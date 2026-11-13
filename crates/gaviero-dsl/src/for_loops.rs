//! `for <var> in <list> parallel|sequential { … }` expansion.
//!
//! Runs after roster expansion, inside
//! [`expand_workflow_params_in_script`](crate::workflow_params::expand_workflow_params_in_script).
//! Each item of the list param and each body agent yields one clone named
//! `<item-id>-<agent>`. Per-item values go into [`AgentDecl::item_vars`]
//! (keys `<var>.id`, `<var>.<field>`, `<var>.index`, …), which the compiler
//! substitutes before every other var layer.
//!
//! - `parallel`: clones of different items are independent.
//! - `sequential`: every clone of item k depends on every clone of item k−1,
//!   and the workflow must run in `execution_mode document` (item k reads
//!   item k−1's files, which repo-mode worktrees would hide).
//!
//! A `for … parallel` inside a `loop` appends its clones to the loop's agents;
//! when the step immediately before the loop is a top-level `for` over the
//! same list, its clones become the loop's `baseline_units`.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::DslError;
use crate::workflow_params::{resolve_roster, roster_entry_tier, synth_client_from_entry};

/// Free-field names that would shadow a derived item var.
const RESERVED_FIELDS: &[&str] = &["index", "count", "peers", "peer_count", "prev", "peer_block"];

fn compile_err(span: Span, reason: String) -> DslError {
    DslError::Compile {
        src: miette::NamedSource::new("script", String::new()),
        span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
        reason,
    }
}

fn valid_item_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Where a `for` block sits in the workflow.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// A top-level step, at this index of `steps`.
    Step(usize),
    /// Inside the `loop` at this index of `steps`.
    InLoop(usize),
}

/// One `for` block with its resolved items, ready to clone.
struct ResolvedFor {
    block: ForBlock,
    placement: Placement,
    items: Vec<ReviewerEntry>,
}

/// Expand every `for` block of the selected workflow(s).
pub(crate) fn expand_for_blocks(
    script: &mut Script,
    workflow_name: Option<&str>,
    override_params: &[(String, String)],
    kinds_by_workflow: &HashMap<String, HashMap<String, ParamKind>>,
    errors: &mut Vec<DslError>,
) {
    let agent_by_name: HashMap<String, AgentDecl> = script
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Agent(a) => Some((a.name.clone(), a.clone())),
            _ => None,
        })
        .collect();
    let prompt_by_name: HashMap<String, String> = script
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Prompt(p) => Some((p.name.clone(), p.content.clone())),
            _ => None,
        })
        .collect();

    let mut new_clients: Vec<ClientDecl> = Vec::new();
    let mut new_agents: Vec<AgentDecl> = Vec::new();
    // body agent name → every clone of it (for fan-in rewriting).
    let mut clones_of: HashMap<String, Vec<String>> = HashMap::new();

    let empty_kinds = HashMap::new();
    for item in script.items.iter_mut() {
        let Item::Workflow(wf) = item else { continue };
        if let Some(name) = workflow_name
            && wf.name != name
        {
            continue;
        }
        let execution_document = matches!(wf.execution, Some((ExecutionMode::Document, _)));
        let resolved_kinds = kinds_by_workflow.get(&wf.name).unwrap_or(&empty_kinds);
        let params_by_name: HashMap<String, ParamDecl> = wf
            .params
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();
        let Some((steps, _)) = wf.steps.as_mut() else {
            continue;
        };

        // ── collect the blocks ────────────────────────────────────────
        let mut blocks: Vec<(ForBlock, Placement)> = Vec::new();
        let mut plain_steps: HashSet<String> = HashSet::new();
        for (i, step) in steps.iter().enumerate() {
            match step {
                StepItem::For(fb) => blocks.push((fb.clone(), Placement::Step(i))),
                StepItem::Loop(lb) => {
                    for fb in &lb.fors {
                        blocks.push((fb.clone(), Placement::InLoop(i)));
                    }
                    plain_steps.extend(lb.agents.iter().map(|(n, _)| n.clone()));
                }
                StepItem::Agent(name, _) => {
                    plain_steps.insert(name.clone());
                }
            }
        }
        if blocks.is_empty() {
            continue;
        }

        // ── validate blocks and resolve their lists ───────────────────
        let mut resolved: Vec<ResolvedFor> = Vec::new();
        let mut body_owner: HashMap<String, usize> = HashMap::new();
        for (idx, (fb, placement)) in blocks.into_iter().enumerate() {
            let mut ok = true;
            if fb.mode == ForMode::Sequential {
                if matches!(placement, Placement::InLoop(_)) {
                    errors.push(compile_err(
                        fb.mode_span,
                        "`for … sequential` is not allowed inside a `loop` — use `parallel` there"
                            .into(),
                    ));
                    ok = false;
                } else if !execution_document {
                    errors.push(compile_err(
                        fb.mode_span,
                        format!(
                            "`for {} in {} sequential` needs the workflow to declare \
                             `execution_mode document` (each item reads the previous item's files)",
                            fb.var, fb.list
                        ),
                    ));
                    ok = false;
                }
            }
            for (agent, span) in &fb.agents {
                let Some(decl) = agent_by_name.get(agent) else {
                    errors.push(compile_err(
                        *span,
                        format!("`for` body agent `{agent}` is not a defined agent"),
                    ));
                    ok = false;
                    continue;
                };
                if plain_steps.contains(agent) {
                    errors.push(compile_err(
                        *span,
                        format!(
                            "agent `{agent}` is the body of a `for` and is also listed as a plain step — \
                             a body agent runs only through its clones"
                        ),
                    ));
                    ok = false;
                }
                if let Some(prev) = body_owner.insert(agent.clone(), idx)
                    && prev != idx
                {
                    errors.push(compile_err(
                        *span,
                        format!("agent `{agent}` is the body of more than one `for` block"),
                    ));
                    ok = false;
                }
                // Disjointness: every clone must write its own files.
                let id_ph = format!("{{{{{}.id}}}}", fb.var);
                let owned = decl.scope.as_ref().map(|s| s.owned.as_slice()).unwrap_or(&[]);
                for path in owned.iter().chain(decl.produces.iter()) {
                    if !path.contains(&id_ph) {
                        errors.push(compile_err(
                            *span,
                            format!(
                                "`for` body agent `{agent}` writes `{path}`, which does not contain \
                                 `{id_ph}` — every clone must own and produce its own files"
                            ),
                        ));
                        ok = false;
                    }
                }
            }

            // Resolve the list param.
            let items = match params_by_name.get(&fb.list) {
                None => {
                    errors.push(compile_err(
                        fb.list_span,
                        format!(
                            "for … in `{}`: `{}` is not a declared workflow param — add \
                             `param {} [ … ]`",
                            fb.list, fb.list, fb.list
                        ),
                    ));
                    None
                }
                Some(_) if !matches!(resolved_kinds.get(&fb.list), Some(ParamKind::Roster(_))) => {
                    errors.push(compile_err(
                        fb.list_span,
                        format!("for … in `{}`: `{}` is not a list param", fb.list, fb.list),
                    ));
                    None
                }
                Some(_) => match resolve_roster(
                    &ReviewerSource::ParamRef(fb.list.clone(), fb.list_span),
                    &params_by_name,
                    resolved_kinds,
                    override_params,
                ) {
                    Ok(items) => Some(items),
                    Err(e) => {
                        errors.push(e);
                        None
                    }
                },
            };
            let Some(items) = items else { continue };
            if !validate_items(&fb, &items, errors) {
                ok = false;
            }
            if ok {
                resolved.push(ResolvedFor {
                    block: fb,
                    placement,
                    items,
                });
            }
        }
        if !errors.is_empty() {
            return;
        }

        // ── clone ────────────────────────────────────────────────────
        // clones per block, item-major order.
        let mut block_clones: Vec<Vec<String>> = Vec::new();
        for rf in &resolved {
            let mut names = Vec::new();
            let ids: Vec<&str> = rf.items.iter().map(|e| e.id.as_str()).collect();
            let body: HashSet<&str> = rf.block.agents.iter().map(|(n, _)| n.as_str()).collect();
            for (k, entry) in rf.items.iter().enumerate() {
                let item_vars = item_vars_for(&rf.block, &rf.items, k);
                for (agent, _) in &rf.block.agents {
                    let template = &agent_by_name[agent];
                    let clone_name = format!("{}-{}", entry.id, agent);
                    let mut clone = template.clone();
                    clone.name = clone_name.clone();
                    clone.template = false;
                    clone.item_vars = item_vars.clone();
                    if entry.model.is_some() {
                        match roster_entry_tier(entry) {
                            Err(reason) => errors.push(compile_err(entry.span, reason)),
                            Ok(Some(tier)) => {
                                clone.client = None;
                                clone.tier_ref = Some((tier.to_string(), template.name_span));
                            }
                            Ok(None) => {
                                let client_name = format!("__for_{}_{}", entry.id, agent);
                                new_clients.push(synth_client_from_entry(&client_name, entry));
                                clone.client = Some((client_name, entry.span));
                                clone.tier_ref = None;
                            }
                        }
                    }
                    // depends_on: same-item siblings, then the sequential chain.
                    let mut deps: Vec<(String, Span)> = Vec::new();
                    let deps_span = template.depends_on.as_ref().map(|(_, s)| *s);
                    if let Some((list, _)) = &template.depends_on {
                        for (dep, span) in list {
                            if body.contains(dep.as_str()) {
                                deps.push((format!("{}-{}", entry.id, dep), *span));
                            } else {
                                deps.push((dep.clone(), *span));
                            }
                        }
                    }
                    if rf.block.mode == ForMode::Sequential && k > 0 {
                        for (prev_agent, _) in &rf.block.agents {
                            let prev = format!("{}-{}", ids[k - 1], prev_agent);
                            if !deps.iter().any(|(d, _)| d == &prev) {
                                deps.push((prev, rf.block.span));
                            }
                        }
                    }
                    clone.depends_on = if deps.is_empty() {
                        None
                    } else {
                        Some((deps, deps_span.unwrap_or(rf.block.span)))
                    };
                    check_unknown_item_vars(&clone, &rf.block.var, &prompt_by_name, errors);
                    clones_of
                        .entry(agent.clone())
                        .or_default()
                        .push(clone_name.clone());
                    names.push(clone_name);
                    new_agents.push(clone);
                }
            }
            block_clones.push(names);
        }

        // ── rewrite the steps ────────────────────────────────────────
        let mut by_step: HashMap<usize, Vec<usize>> = HashMap::new();
        for (bi, rf) in resolved.iter().enumerate() {
            let (Placement::Step(i) | Placement::InLoop(i)) = rf.placement;
            by_step.entry(i).or_default().push(bi);
        }
        let old_steps = std::mem::take(steps);
        let mut new_steps: Vec<StepItem> = Vec::new();
        // The baseline of a loop is the top-level `for` step *immediately*
        // before it, when that step iterates the same list (the
        // `for r in roster parallel { init }` + `loop { for r in roster … }`
        // shape). Earlier `for` steps are ordinary upstream work.
        let mut previous_for: Option<(String, Vec<String>)> = None;
        for (i, step) in old_steps.into_iter().enumerate() {
            match step {
                StepItem::For(_) => {
                    for &bi in by_step.get(&i).map(Vec::as_slice).unwrap_or(&[]) {
                        let rf = &resolved[bi];
                        for name in &block_clones[bi] {
                            new_steps.push(StepItem::Agent(name.clone(), rf.block.span));
                        }
                        previous_for = Some((rf.block.list.clone(), block_clones[bi].clone()));
                    }
                }
                StepItem::Loop(mut lb) => {
                    for &bi in by_step.get(&i).map(Vec::as_slice).unwrap_or(&[]) {
                        let rf = &resolved[bi];
                        for name in &block_clones[bi] {
                            lb.agents.push((name.clone(), rf.block.span));
                        }
                        if let Some((list, base)) = &previous_for
                            && *list == rf.block.list
                        {
                            for unit in base {
                                if !lb.baseline_units.contains(unit) {
                                    lb.baseline_units.push(unit.clone());
                                }
                            }
                        }
                    }
                    lb.fors.clear();
                    previous_for = None;
                    new_steps.push(StepItem::Loop(lb));
                }
                other => {
                    previous_for = None;
                    new_steps.push(other);
                }
            }
        }
        *steps = new_steps;
    }

    if !errors.is_empty() || clones_of.is_empty() {
        return;
    }

    // ── fan-in: `depends_on [body]` outside the body → every clone ──
    let body_names: HashSet<String> = clones_of.keys().cloned().collect();
    for item in script.items.iter_mut() {
        let Item::Agent(agent) = item else { continue };
        if body_names.contains(&agent.name) {
            continue;
        }
        if let Some((deps, _)) = agent.depends_on.as_mut() {
            let mut rewritten: Vec<(String, Span)> = Vec::new();
            for (dep, span) in deps.drain(..) {
                match clones_of.get(&dep) {
                    Some(clones) => {
                        rewritten.extend(clones.iter().map(|c| (c.clone(), span)));
                    }
                    None => rewritten.push((dep, span)),
                }
            }
            *deps = rewritten;
        }
    }
    // Clones depending on a body agent of another `for` fan in as well.
    for clone in new_agents.iter_mut() {
        if let Some((deps, _)) = clone.depends_on.as_mut() {
            let mut rewritten: Vec<(String, Span)> = Vec::new();
            for (dep, span) in deps.drain(..) {
                match clones_of.get(&dep) {
                    Some(clones) => rewritten.extend(clones.iter().map(|c| (c.clone(), span))),
                    None => rewritten.push((dep, span)),
                }
            }
            *deps = rewritten;
        }
    }

    for client in new_clients {
        script.items.push(Item::Client(client));
    }
    for agent in new_agents {
        script.items.push(Item::Agent(agent));
    }
}

/// Check ids and fields of a resolved list. Returns `false` on any error.
fn validate_items(fb: &ForBlock, items: &[ReviewerEntry], errors: &mut Vec<DslError>) -> bool {
    let before = errors.len();
    if items.is_empty() {
        errors.push(compile_err(
            fb.list_span,
            format!("for … in `{}`: the list is empty", fb.list),
        ));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for entry in items {
        if entry.id.is_empty() {
            errors.push(compile_err(
                entry.span,
                format!("list `{}`: every entry needs an `id`", fb.list),
            ));
        } else if !valid_item_id(&entry.id) {
            errors.push(compile_err(
                entry.id_span,
                format!(
                    "list `{}`: id `{}` is invalid — use letters, digits, `_`, `.` or `-`, \
                     starting with a letter or digit",
                    fb.list, entry.id
                ),
            ));
        } else if !seen.insert(entry.id.as_str()) {
            errors.push(compile_err(
                entry.id_span,
                format!("list `{}`: duplicate id `{}`", fb.list, entry.id),
            ));
        }
        let mut names: HashSet<&str> = HashSet::new();
        for (name, _, span) in &entry.fields {
            if RESERVED_FIELDS.contains(&name.as_str()) {
                errors.push(compile_err(
                    *span,
                    format!(
                        "list `{}`: field `{name}` is reserved (it is a derived `{}.{name}` var)",
                        fb.list, fb.var
                    ),
                ));
            } else if matches!(name.as_str(), "id" | "model" | "effort") || !names.insert(name) {
                errors.push(compile_err(
                    *span,
                    format!(
                        "list `{}`: duplicate field `{name}` in entry `{}`",
                        fb.list, entry.id
                    ),
                ));
            }
        }
    }
    errors.len() == before
}

/// The item vars of item `k`, keyed `<var>.<name>`.
fn item_vars_for(fb: &ForBlock, items: &[ReviewerEntry], k: usize) -> Vec<(String, String)> {
    let var = &fb.var;
    let entry = &items[k];
    let key = |name: &str| format!("{var}.{name}");
    let peers: Vec<&str> = items
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != k)
        .map(|(_, e)| e.id.as_str())
        .collect();
    let mut vars = vec![(key("id"), entry.id.clone())];
    for (name, value, _) in &entry.fields {
        vars.push((key(name), value.clone()));
    }
    if let Some(model) = &entry.model {
        vars.push((key("model"), model.clone()));
    }
    if let Some((effort, _)) = &entry.effort {
        vars.push((key("effort"), effort.clone()));
    }
    vars.push((key("index"), (k + 1).to_string()));
    vars.push((key("count"), items.len().to_string()));
    vars.push((key("peers"), peers.join(",")));
    vars.push((key("peer_count"), peers.len().to_string()));
    if fb.mode == ForMode::Sequential {
        let prev = if k == 0 { String::new() } else { items[k - 1].id.clone() };
        vars.push((key("prev"), prev));
    }
    if let Some((template, _)) = &fb.peer_template {
        let block = if peers.is_empty() {
            "(none)".to_string()
        } else {
            peers
                .iter()
                .map(|p| template.replace("{{peer}}", p))
                .collect::<Vec<_>>()
                .join("\n")
        };
        vars.push((key("peer_block"), block));
    }
    vars
}

/// Report `{{<var>.<name>}}` placeholders in a clone that no item var fills.
fn check_unknown_item_vars(
    clone: &AgentDecl,
    var: &str,
    prompts: &HashMap<String, String>,
    errors: &mut Vec<DslError>,
) {
    let known: HashSet<&str> = clone.item_vars.iter().map(|(k, _)| k.as_str()).collect();
    let mut texts: Vec<&str> = Vec::new();
    if let Some((d, _)) = &clone.description {
        texts.push(d);
    }
    match &clone.prompt {
        Some((PromptSource::Inline(s), _)) => texts.push(s),
        Some((PromptSource::Ref(name, _), _)) => {
            if let Some(s) = prompts.get(name) {
                texts.push(s);
            }
        }
        None => {}
    }
    if let Some(scope) = &clone.scope {
        texts.extend(scope.owned.iter().map(String::as_str));
        texts.extend(scope.read_only.iter().map(String::as_str));
    }
    texts.extend(clone.produces.iter().map(String::as_str));
    if let Some(memory) = &clone.memory {
        if let Some((s, _)) = &memory.write_content {
            texts.push(s);
        }
        if let Some((s, _)) = &memory.read_query {
            texts.push(s);
        }
    }

    let open = format!("{{{{{var}.");
    let mut reported: HashSet<String> = HashSet::new();
    for text in texts {
        let mut rest = text;
        while let Some(pos) = rest.find(&open) {
            let after = &rest[pos + 2..];
            let Some(end) = after.find("}}") else { break };
            let key = &after[..end];
            if !known.contains(key) && reported.insert(key.to_string()) {
                let field = key.strip_prefix(&format!("{var}.")).unwrap_or(key);
                let hint = if field == "peer_block" {
                    " — declare a `peer_template` in the `for` body".to_string()
                } else if field == "prev" {
                    " — `prev` exists only in `for … sequential`".to_string()
                } else {
                    String::new()
                };
                errors.push(compile_err(
                    clone.span,
                    format!(
                        "agent `{}` uses `{{{{{key}}}}}`, but `{var}` has no field `{field}`{hint}",
                        clone.name
                    ),
                ));
            }
            rest = &after[end + 2..];
        }
    }
}
