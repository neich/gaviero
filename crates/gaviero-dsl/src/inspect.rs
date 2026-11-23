//! What a script asks of whoever runs it, read without compiling.
//!
//! A front end needs this before it has values to compile with: required
//! params are a compile error until they are supplied.

use std::collections::HashSet;
use std::path::Path;

use crate::ast::{Item, PatternDecl, Script, StepItem, VarKind, WorkflowDecl};
use crate::compiler::peek_workflow_execution_mode;
use crate::resolver;
use crate::workflow_params::describe_workflow_params;
use gaviero_core::swarm::plan::ExecutionMode;

/// Placeholders the compiler or runtime fills in, which a caller cannot set.
const RESERVED_PLACEHOLDERS: &[&str] = &[
    "PROMPT",
    "AGENT",
    "ITER",
    "PREV_ITER",
    "REVIEWER_ID",
    "ITER_EVIDENCE",
    "GATE_FEEDBACK",
    "SUMMARY",
    "DESCRIPTION",
    "FILES",
];

/// A script's inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInspection {
    pub workflows: Vec<WorkflowInspection>,
    /// Script-level `vars {}`, then `{{KEY}}` placeholders nothing declares.
    pub vars: Vec<VarInspection>,
    /// `{{PROMPT}}` appears: without a value it reaches agents literally.
    pub requires_prompt: bool,
    /// Runtime prompt has an effect: `{{PROMPT}}`, or an agent with no
    /// `prompt` field, which takes it as its instructions.
    pub accepts_prompt: bool,
    /// Execution mode when the script declares no workflow.
    pub default_execution: ExecutionMode,
    /// Tiers the script asks for (agents, `tiers { }` blocks, list items),
    /// in order of appearance. The profile must define each of them.
    pub tiers: Vec<String>,
    /// Every agent, in declaration order, with the tier it declares.
    pub agents: Vec<AgentInspection>,
}

/// An agent and the tier it declares (`None` only in a script that does not
/// compile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInspection {
    pub name: String,
    pub tier: Option<String>,
}

impl ScriptInspection {
    /// The vars declared as `file`.
    pub fn file_var_names(&self) -> Vec<String> {
        self.vars
            .iter()
            .filter(|v| v.kind == VarKind::File)
            .map(|v| v.name.clone())
            .collect()
    }

    /// The vars declared as `dir`.
    pub fn dir_var_names(&self) -> Vec<String> {
        self.vars
            .iter()
            .filter(|v| v.kind == VarKind::Dir)
            .map(|v| v.name.clone())
            .collect()
    }

    /// `vars` plus the defaults of `file` vars it does not set. Run
    /// preparation only sees what it is given, and a file must be brought
    /// within reach of agents whether or not the user changed it.
    pub fn with_file_defaults(&self, mut vars: Vec<(String, String)>) -> Vec<(String, String)> {
        for var in self.vars.iter().filter(|v| v.kind == VarKind::File) {
            if let Some(default) = &var.default
                && !vars.iter().any(|(k, _)| *k == var.name)
            {
                vars.push((var.name.clone(), default.clone()));
            }
        }
        vars
    }

    /// The agents a run of workflow `workflow` (an index into
    /// [`Self::workflows`]) dispatches, in declaration order, each with the
    /// tier it runs on unless the run moves it: the workflow's `tiers { }`
    /// entry, else its own. With no such workflow, or one whose agents cannot
    /// be told from its steps, every agent is listed.
    pub fn agent_tiers(&self, workflow: usize) -> Vec<(String, String)> {
        let wf = self.workflows.get(workflow);
        let used = wf.map(|w| w.agents.as_slice()).unwrap_or_default();
        self.agents
            .iter()
            .filter(|a| used.is_empty() || used.contains(&a.name))
            .map(|a| {
                let moved = wf.and_then(|w| w.tiers.iter().find(|(agent, _)| *agent == a.name));
                let tier = moved
                    .map(|(_, t)| t.clone())
                    .or_else(|| a.tier.clone())
                    .unwrap_or_default();
                (a.name.clone(), tier)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInspection {
    pub name: String,
    pub execution: ExecutionMode,
    pub params: Vec<ParamInspection>,
    /// Agents its steps, loops, judges, `for` bodies and pattern name, in
    /// order of appearance; empty when it has no steps.
    pub agents: Vec<String>,
    /// Its `tiers { <agent> <tier> }` block.
    pub tiers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarInspection {
    pub name: String,
    /// `None` for a placeholder that no `vars {}` block declares, or a
    /// `file` var the caller must supply.
    pub default: Option<String>,
    pub kind: VarKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKindInfo {
    /// A list: `id=tier,…`, bare ids `a,b`, or a list literal `[{id "a" …}]`.
    Roster,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamInspection {
    pub name: String,
    pub kind: ParamKindInfo,
    /// The in-script default in `--param` syntax; `None` means required.
    pub default: Option<String>,
}

/// The tiers a profile file binds, in declaration order.
pub fn profile_tier_names(profile_path: &Path) -> Result<Vec<String>, miette::Report> {
    let (profile, _) = resolver::resolve(profile_path).map_err(miette::Report::new)?;
    Ok(tier_names(&profile))
}

/// What a model profile offers: its clients and which one each tier means.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileInspection {
    /// In declaration order.
    pub clients: Vec<ProfileClient>,
    /// `(tier, client)`, in declaration order.
    pub tiers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileClient {
    pub name: String,
    /// `provider:model`.
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl ProfileInspection {
    /// The client tier `tier` is bound to.
    pub fn client_of(&self, tier: &str) -> Option<&str> {
        self.tiers
            .iter()
            .find(|(t, _)| t == tier)
            .map(|(_, c)| c.as_str())
    }

    pub fn client(&self, name: &str) -> Option<&ProfileClient> {
        self.clients.iter().find(|c| c.name == name)
    }

    pub fn tier_names(&self) -> Vec<String> {
        self.tiers.iter().map(|(t, _)| t.clone()).collect()
    }
}

impl ProfileClient {
    /// `claude:opus · high`, or the client's name when it has no model.
    pub fn summary(&self) -> String {
        match (&self.model, &self.effort) {
            (Some(model), Some(effort)) => format!("{model} · {effort}"),
            (Some(model), None) => model.clone(),
            _ => self.name.clone(),
        }
    }
}

/// Read a profile file (its includes resolved) without compiling a script.
pub fn inspect_profile(profile_path: &Path) -> Result<ProfileInspection, miette::Report> {
    let (profile, _) = resolver::resolve(profile_path).map_err(miette::Report::new)?;
    let mut info = ProfileInspection::default();
    for item in &profile.items {
        match item {
            Item::Client(c) if info.client(&c.name).is_none() => {
                info.clients.push(ProfileClient {
                    name: c.name.clone(),
                    model: c.model.as_ref().map(|(m, _)| m.clone()),
                    effort: c.effort.as_ref().map(|(e, _)| e.clone()),
                });
            }
            Item::TierAlias(t) if info.client_of(&t.name).is_none() => {
                info.tiers.push((t.name.clone(), t.client_ref.clone()));
            }
            _ => {}
        }
    }
    Ok(info)
}

fn tier_names(script: &Script) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for item in &script.items {
        if let Item::TierAlias(tier) = item
            && !names.contains(&tier.name)
        {
            names.push(tier.name.clone());
        }
    }
    names
}

/// The tiers a script asks for, in order of appearance: every agent's
/// `tier`, every workflow `tiers { }` entry and every list item's `tier`.
fn used_tier_names(script: &Script) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    };
    for item in &script.items {
        match item {
            Item::Agent(agent) => {
                if let Some((tier, _)) = &agent.tier_ref {
                    add(tier);
                }
            }
            Item::Workflow(wf) => {
                for entry in &wf.tiers {
                    add(&entry.tier);
                }
                for param in &wf.params {
                    if let Some(crate::ast::ParamShape::Roster(entries)) = &param.shape {
                        for (tier, _) in entries.iter().filter_map(|e| e.tier.as_ref()) {
                            add(tier);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    names
}

fn push_unique(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
}

/// The agents a workflow dispatches, as its steps name them.
fn workflow_agents(wf: &WorkflowDecl) -> Vec<String> {
    let mut names = Vec::new();
    for step in wf.steps.iter().flat_map(|(steps, _)| steps) {
        match step {
            StepItem::Agent(name, _) => push_unique(&mut names, name),
            StepItem::For(block) => {
                for (name, _) in &block.agents {
                    push_unique(&mut names, name);
                }
            }
            StepItem::Loop(block) => {
                for (name, _) in &block.agents {
                    push_unique(&mut names, name);
                }
                for (name, _) in block.fors.iter().flat_map(|f| &f.agents) {
                    push_unique(&mut names, name);
                }
                for (name, _) in block.template_init.iter().chain(&block.template_refine) {
                    push_unique(&mut names, name);
                }
                for (name, _) in block.until.agents() {
                    push_unique(&mut names, name);
                }
            }
        }
    }
    if let Some(PatternDecl::MapReduce(pattern)) = &wf.pattern {
        push_unique(&mut names, &pattern.discover.0);
        push_unique(&mut names, &pattern.reduce.0);
    }
    names
}

/// Inspect a script on disk, resolving its includes.
pub fn inspect_file(entry_path: &Path) -> Result<ScriptInspection, miette::Report> {
    let (script, sources) = resolver::resolve(entry_path).map_err(miette::Report::new)?;
    Ok(inspect_script(&script, &sources))
}

fn inspect_script(script: &Script, sources: &[(String, String)]) -> ScriptInspection {
    let workflows = script
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Workflow(wf) => Some(WorkflowInspection {
                name: wf.name.clone(),
                execution: peek_workflow_execution_mode(script, Some(&wf.name)),
                params: describe_workflow_params(script, wf),
                agents: workflow_agents(wf),
                tiers: wf
                    .tiers
                    .iter()
                    .map(|t| (t.agent.clone(), t.tier.clone()))
                    .collect(),
            }),
            _ => None,
        })
        .collect();

    let mut declared: Vec<VarInspection> = Vec::new();
    let mut agent_vars: HashSet<String> = HashSet::new();
    let mut agent_without_prompt = false;
    for item in &script.items {
        match item {
            Item::Vars(decls) => {
                for decl in decls {
                    let var = VarInspection {
                        name: decl.name.clone(),
                        default: decl.default.clone(),
                        kind: decl.kind,
                    };
                    match declared.iter_mut().find(|v| v.name == decl.name) {
                        Some(existing) => *existing = var,
                        None => declared.push(var),
                    }
                }
            }
            Item::Agent(agent) => {
                agent_vars.extend(agent.vars.iter().map(|(k, _)| k.clone()));
                agent_without_prompt |= agent.prompt.is_none();
            }
            _ => {}
        }
    }

    let placeholders = placeholders_in(sources);
    let requires_prompt = placeholders.iter().any(|p| p == "PROMPT");

    let mut vars = declared;
    for name in placeholders {
        let settable = !RESERVED_PLACEHOLDERS.contains(&name.as_str())
            && !agent_vars.contains(&name)
            && !vars.iter().any(|v| v.name == name);
        if settable {
            vars.push(VarInspection {
                name,
                default: None,
                kind: VarKind::String,
            });
        }
    }

    ScriptInspection {
        workflows,
        vars,
        requires_prompt,
        accepts_prompt: requires_prompt || agent_without_prompt,
        default_execution: peek_workflow_execution_mode(script, None),
        tiers: used_tier_names(script),
        agents: script
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Agent(agent) => Some(AgentInspection {
                    name: agent.name.clone(),
                    tier: agent.tier_ref.as_ref().map(|(t, _)| t.clone()),
                }),
                _ => None,
            })
            .collect(),
    }
}

/// Every distinct `{{KEY}}` outside comment lines, in order of appearance.
fn placeholders_in(sources: &[(String, String)]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (_, source) in sources {
        for line in source.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(open) = rest.find("{{") {
                rest = &rest[open + 2..];
                let Some(close) = rest.find("}}") else { break };
                let key = &rest[..close];
                let is_key = !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if is_key && !found.iter().any(|f| f == key) {
                    found.push(key.to_string());
                }
                if is_key {
                    rest = &rest[close + 2..];
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect(source: &str) -> ScriptInspection {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.gaviero");
        std::fs::write(&path, source).unwrap();
        inspect_file(&path).unwrap()
    }

    #[test]
    fn declared_vars_keep_their_defaults_and_unknown_placeholders_are_listed() {
        let info = inspect(concat!(
            "vars { OUT_DIR \"plans/out\" }\n",
            "agent a { tier t\n",
            "  scope { owned [\"{{OUT_DIR}}/{{TARGET}}.md\"] }\n",
            "  prompt #\"work on {{TARGET}}\"#\n",
            "}\n",
        ));
        assert_eq!(
            info.vars,
            vec![
                VarInspection {
                    name: "OUT_DIR".into(),
                    default: Some("plans/out".into()),
                    kind: VarKind::String,
                },
                VarInspection {
                    name: "TARGET".into(),
                    default: None,
                    kind: VarKind::String,
                },
            ]
        );
        assert!(!info.requires_prompt);
        assert!(!info.accepts_prompt, "every agent has its own prompt");
    }

    #[test]
    fn reserved_commented_and_agent_level_placeholders_are_not_inputs() {
        let info = inspect(concat!(
            "// mentions {{IN_A_COMMENT}}\n",
            "agent a { tier t\n",
            "  vars { LOCAL \"x\" }\n",
            "  prompt #\"{{PROMPT}} {{ITER}} {{AGENT}} {{LOCAL}}\"#\n",
            "}\n",
        ));
        assert!(info.vars.is_empty(), "{:?}", info.vars);
        assert!(info.requires_prompt);
        assert!(info.accepts_prompt);
    }

    #[test]
    fn file_vars_report_their_kind_and_only_a_default_can_be_added() {
        let info = inspect(concat!(
            "vars {\n",
            "  PLAIN \"x\"\n",
            "  INPUT file \"docs/a.md\"\n",
            "  NEEDED file\n",
            "  AFTER \"y\"\n",
            "}\n",
            "agent a { tier t prompt #\"{{INPUT}} {{NEEDED}}\"# }\n",
        ));
        let names: Vec<&str> = info.vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["PLAIN", "INPUT", "NEEDED", "AFTER"]);
        assert_eq!(info.file_var_names(), ["INPUT", "NEEDED"]);
        assert_eq!(info.vars[0].kind, VarKind::String);
        assert_eq!(info.vars[2].default, None);

        let given = vec![("OTHER".to_string(), "1".to_string())];
        assert_eq!(
            info.with_file_defaults(given),
            vec![
                ("OTHER".to_string(), "1".to_string()),
                ("INPUT".to_string(), "docs/a.md".to_string()),
            ]
        );
        let overridden = vec![("INPUT".to_string(), "b.md".to_string())];
        assert_eq!(info.with_file_defaults(overridden.clone()), overridden);
    }

    const NEEDS_FILE: &str = concat!(
        "vars { NEEDED file\n PLAIN \"p\" }\n",
        "agent a { tier t scope { owned [\"o.md\"] } prompt #\"read {{NEEDED}} {{PLAIN}}\"# }\n",
        "workflow w { steps [a] }\n",
    );

    /// `crate::compile_with_vars` against `tests/fixtures/test-profile.gaviero`.
    fn compile_with_vars(
        src: &str,
        vars: &[(String, String)],
    ) -> Result<gaviero_core::swarm::plan::CompiledPlan, miette::Report> {
        let profile = gaviero_core::script_profiles::SelectedProfile {
            name: "test".into(),
            path: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-profile.gaviero"),
            precedence: gaviero_core::script_profiles::ProfilePrecedence::OverridesScript,
        };
        crate::compile_with_vars(src, "t", Some(&profile), None, None, vars, &[], &[])
    }

    #[test]
    fn a_file_var_without_a_default_must_be_supplied() {
        let err = compile_with_vars(NEEDS_FILE, &[]).unwrap_err();
        assert!(format!("{err:?}").contains("file var `NEEDED`"), "{err:?}");

        let vars = vec![("NEEDED".to_string(), "docs/a.md".to_string())];
        let plan = compile_with_vars(NEEDS_FILE, &vars).unwrap();
        let shown = format!("{plan:?}");
        assert!(shown.contains("read docs/a.md p"), "{shown}");
    }

    #[test]
    fn dir_vars_are_listed_apart_from_files_and_can_be_required() {
        let info = inspect(concat!(
            "vars {\n  SRC dir \"src\"\n  DATA dir\n  IN file \"a.md\"\n}\n",
            "agent a { tier t prompt #\"{{SRC}} {{DATA}} {{IN}}\"# }\n",
        ));
        assert_eq!(info.dir_var_names(), ["SRC", "DATA"]);
        assert_eq!(info.file_var_names(), ["IN"]);
        assert_eq!(
            info.with_file_defaults(Vec::new()),
            vec![("IN".to_string(), "a.md".to_string())],
            "a dir default is not an override"
        );

        let src = NEEDS_FILE.replace("NEEDED file\n", "NEEDED dir\n");
        let err = compile_with_vars(&src, &[]).unwrap_err();
        assert!(format!("{err:?}").contains("dir var `NEEDED`"), "{err:?}");
    }

    #[test]
    fn untyped_vars_compile_exactly_as_before() {
        let src = NEEDS_FILE.replace("NEEDED file\n", "NEEDED \"x.md\"\n");
        let plan = compile_with_vars(&src, &[]).unwrap();
        assert!(format!("{plan:?}").contains("read x.md p"));
    }

    #[test]
    fn agent_level_vars_take_no_type() {
        let src = "agent a { tier t vars { X file \"y\" } prompt #\"{{X}}\"# }\n";
        assert!(compile_with_vars(src, &[]).is_err());
    }

    #[test]
    fn used_and_profile_tiers_are_listed_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("p.gaviero");
        std::fs::write(
            &profile,
            "client a { model \"claude:opus\" privacy public }\ntier top a\ntier low a\n",
        )
        .unwrap();
        assert_eq!(profile_tier_names(&profile).unwrap(), ["top", "low"]);

        // A script lists the tiers it asks for: agents, then the workflow's
        // `tiers { }` block and list items, deduplicated.
        let info = inspect(concat!(
            "agent a { tier mine prompt #\"x\"# }\n",
            "agent b { tier low prompt #\"x\"# }\n",
            "workflow w {\n",
            "  tiers { a top }\n",
            "  param l [ { id \"x\" tier mine } { id \"y\" tier high } ]\n",
            "  steps [ a b ]\n",
            "}\n",
        ));
        assert_eq!(info.tiers, ["mine", "low", "top", "high"]);
    }

    #[test]
    fn agents_are_listed_with_the_tier_each_workflow_runs_them_on() {
        let info = inspect(concat!(
            "agent scout { tier high prompt #\"x\"# }\n",
            "agent unused { tier low prompt #\"x\"# }\n",
            "agent worker { tier high prompt #\"x\"# }\n",
            "agent judge { tier medium prompt #\"x\"# }\n",
            "workflow w {\n",
            "  tiers { judge top }\n",
            "  steps [ scout loop { agents [worker] until agent judge } ]\n",
            "}\n",
            "workflow all { steps [ unused ] }\n",
        ));
        assert_eq!(info.workflows[0].agents, ["scout", "worker", "judge"]);
        let tiers = info.agent_tiers(0);
        let shown: Vec<(&str, &str)> =
            tiers.iter().map(|(a, t)| (a.as_str(), t.as_str())).collect();
        assert_eq!(shown, [("scout", "high"), ("worker", "high"), ("judge", "top")]);
        assert_eq!(info.agent_tiers(1), [("unused".to_string(), "low".to_string())]);
        assert_eq!(info.agent_tiers(9).len(), 4, "no such workflow lists every agent");
    }

    #[test]
    fn the_deep_explore_example_lists_the_agents_of_each_workflow_including_the_for_body() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/deep-explore.gaviero");
        let info = inspect_file(&path).unwrap();
        let names = |workflow: usize| -> Vec<String> {
            info.agent_tiers(workflow).into_iter().map(|(a, _)| a).collect()
        };
        let explore = names(0);
        for agent in ["scout", "curator", "investigator", "verifier"] {
            assert!(explore.iter().any(|n| n == agent), "{agent} missing from {explore:?}");
        }
        assert!(!explore.iter().any(|n| n == "synthesize"), "{explore:?}");
        assert_eq!(names(1), ["synthesize"]);
    }

    #[test]
    fn a_profile_reports_its_clients_and_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("p.gaviero");
        std::fs::write(
            &profile,
            concat!(
                "client opus { model \"claude:opus\" privacy public effort high }\n",
                "client sol { model \"codex:gpt-6.1-sol\" privacy public }\n",
                "tier high opus\ntier high-codex sol\n",
            ),
        )
        .unwrap();
        let info = inspect_profile(&profile).unwrap();
        assert_eq!(info.tier_names(), ["high", "high-codex"]);
        assert_eq!(info.client_of("high-codex"), Some("sol"));
        assert_eq!(info.client("opus").unwrap().summary(), "claude:opus · high");
        assert_eq!(info.client("sol").unwrap().summary(), "codex:gpt-6.1-sol");
    }

    #[test]
    fn agent_without_a_prompt_accepts_one_without_requiring_it() {
        let info = inspect("agent a { tier t description \"do it\" }\n");
        assert!(info.accepts_prompt);
        assert!(!info.requires_prompt);
    }

    #[test]
    fn params_report_kind_and_default_in_override_syntax() {
        let info = inspect(concat!(
            "agent worker { tier t prompt #\"w\"# }\n",
            "agent judge { tier t prompt #\"j\"# }\n",
            "workflow w {\n",
            "  param panel [ { id \"a\" tier high } { id \"b\" } ]\n",
            "  param bare\n",
            "  steps [ loop { agents [worker] reviewers bare until agent judge } for x in panel parallel { judge } ]\n",
            "}\n",
        ));
        let params = &info.workflows[0].params;
        assert_eq!(params[0].default.as_deref(), Some("a=high,b"));
        assert_eq!(params[0].kind, ParamKindInfo::Roster);
        assert_eq!(
            params[1],
            ParamInspection {
                name: "bare".into(),
                kind: ParamKindInfo::Roster,
                default: None,
            }
        );
    }

    #[test]
    fn a_list_default_with_free_fields_is_a_literal_that_parses_back() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/for_loops.gaviero");
        let info = inspect_file(&path).unwrap();
        let topics = &info.workflows[0].params[0];
        assert_eq!(topics.kind, ParamKindInfo::Roster);
        let default = topics.default.as_deref().unwrap();
        assert_eq!(
            default,
            r#"[{id "a" focus "storage"} {id "b" focus "network"} {id "c" tier high focus "ui"}]"#
        );
        let entries = crate::workflow_params::parse_reviewers_override(default).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2].fields[0].1, "ui");

        let mut odd = entries[0].clone();
        odd.fields[0].1 = "say \"hi\"".into();
        odd.tier = Some(("high-codex".into(), (0..0).into()));
        let literal = crate::workflow_params::list_literal_entry(&odd);
        let back = crate::workflow_params::parse_reviewers_override(&format!("[{literal}]")).unwrap();
        assert_eq!(back[0].fields[0].1, "say \"hi\"");
        assert_eq!(back[0].tier.as_ref().map(|(t, _)| t.as_str()), Some("high-codex"));
    }

    #[test]
    fn each_workflow_reports_its_execution_mode() {
        let info = inspect(concat!(
            "agent a { tier t prompt #\"x\"# }\n",
            "workflow one { steps [a] }\n",
            "workflow two { execution_mode document steps [a] }\n",
        ));
        assert_eq!(info.workflows[0].execution, ExecutionMode::Repo);
        assert_eq!(info.workflows[1].execution, ExecutionMode::Document);
    }

    #[test]
    fn every_shipped_workflow_example_can_be_inspected() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut inspected = 0;
        for entry in std::fs::read_dir(&examples).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gaviero") {
                inspect_file(&path).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
                inspected += 1;
            }
        }
        assert!(inspected > 0);
    }

    #[test]
    fn included_files_contribute_inputs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.gaviero"),
            "agent b { tier t prompt #\"{{SHARED}}\"# }\n",
        )
        .unwrap();
        let path = dir.path().join("script.gaviero");
        std::fs::write(&path, "include \"lib.gaviero\"\n").unwrap();

        let info = inspect_file(&path).unwrap();
        assert_eq!(info.vars[0].name, "SHARED");
    }
}
