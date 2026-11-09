//! What a script asks of whoever runs it, read without compiling.
//!
//! A front end needs this before it has values to compile with: required
//! params are a compile error until they are supplied.

use std::collections::HashSet;
use std::path::Path;

use crate::ast::{Item, Script, VarKind};
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
    /// Tiers the script binds itself, in declaration order.
    pub tiers: Vec<String>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInspection {
    pub name: String,
    pub execution: ExecutionMode,
    pub params: Vec<ParamInspection>,
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
    /// `id=provider:model[@effort],…`
    Roster,
    /// `provider:model[@effort]`
    Client,
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
        tiers: tier_names(script),
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
            "agent a {\n",
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
            "agent a {\n",
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
            "agent a { prompt #\"{{INPUT}} {{NEEDED}}\"# }\n",
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
        "client c { model \"claude:opus\" privacy public }\n",
        "agent a { client c scope { owned [\"o.md\"] } prompt #\"read {{NEEDED}} {{PLAIN}}\"# }\n",
        "workflow w { steps [a] }\n",
    );

    #[test]
    fn a_file_var_without_a_default_must_be_supplied() {
        let err = crate::compile_with_vars(NEEDS_FILE, "t", None, None, &[], &[], &[]).unwrap_err();
        assert!(format!("{err:?}").contains("file var `NEEDED`"), "{err:?}");

        let vars = vec![("NEEDED".to_string(), "docs/a.md".to_string())];
        let plan = crate::compile_with_vars(NEEDS_FILE, "t", None, None, &vars, &[], &[]).unwrap();
        let shown = format!("{plan:?}");
        assert!(shown.contains("read docs/a.md p"), "{shown}");
    }

    #[test]
    fn dir_vars_are_listed_apart_from_files_and_can_be_required() {
        let info = inspect(concat!(
            "vars {\n  SRC dir \"src\"\n  DATA dir\n  IN file \"a.md\"\n}\n",
            "agent a { prompt #\"{{SRC}} {{DATA}} {{IN}}\"# }\n",
        ));
        assert_eq!(info.dir_var_names(), ["SRC", "DATA"]);
        assert_eq!(info.file_var_names(), ["IN"]);
        assert_eq!(
            info.with_file_defaults(Vec::new()),
            vec![("IN".to_string(), "a.md".to_string())],
            "a dir default is not an override"
        );

        let src = NEEDS_FILE.replace("NEEDED file\n", "NEEDED dir\n");
        let err = crate::compile_with_vars(&src, "t", None, None, &[], &[], &[]).unwrap_err();
        assert!(format!("{err:?}").contains("dir var `NEEDED`"), "{err:?}");
    }

    #[test]
    fn untyped_vars_compile_exactly_as_before() {
        let src = NEEDS_FILE.replace("NEEDED file\n", "NEEDED \"x.md\"\n");
        let plan = crate::compile_with_vars(&src, "t", None, None, &[], &[], &[]).unwrap();
        assert!(format!("{plan:?}").contains("read x.md p"));
    }

    #[test]
    fn agent_level_vars_take_no_type() {
        let src = concat!(
            "client c { model \"claude:opus\" privacy public }\n",
            "agent a { client c vars { X file \"y\" } prompt #\"{{X}}\"# }\n",
        );
        assert!(crate::compile_with_vars(src, "t", None, None, &[], &[], &[]).is_err());
    }

    #[test]
    fn script_and_profile_tiers_are_listed_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("p.gaviero");
        std::fs::write(
            &profile,
            "client a { model \"claude:opus\" privacy public }\ntier top a\ntier low a\n",
        )
        .unwrap();
        assert_eq!(profile_tier_names(&profile).unwrap(), ["top", "low"]);

        let info = inspect(concat!(
            "client c { model \"claude:opus\" privacy public }\n",
            "tier mine c\n",
            "agent a { tier mine prompt #\"x\"# }\n",
        ));
        assert_eq!(info.tiers, ["mine"]);
    }

    #[test]
    fn agent_without_a_prompt_accepts_one_without_requiring_it() {
        let info = inspect("agent a { description \"do it\" }\n");
        assert!(info.accepts_prompt);
        assert!(!info.requires_prompt);
    }

    #[test]
    fn params_report_kind_and_default_in_override_syntax() {
        let info = inspect(concat!(
            "agent worker { prompt #\"w\"# }\n",
            "agent judge { client gate prompt #\"j\"# }\n",
            "workflow w {\n",
            "  param gate { model \"claude:sonnet\" effort high }\n",
            "  param panel [ { id \"a\" model \"claude:opus\" } { id \"b\" model \"codex:x\" effort low } ]\n",
            "  param bare\n",
            "  steps [ loop { agents [worker] reviewers bare until agent judge } judge ]\n",
            "}\n",
        ));
        let params = &info.workflows[0].params;
        assert_eq!(
            params[0],
            ParamInspection {
                name: "gate".into(),
                kind: ParamKindInfo::Client,
                default: Some("claude:sonnet@high".into()),
            }
        );
        assert_eq!(
            params[1].default.as_deref(),
            Some("a=claude:opus,b=codex:x@low")
        );
        assert_eq!(params[1].kind, ParamKindInfo::Roster);
        assert_eq!(
            params[2],
            ParamInspection {
                name: "bare".into(),
                kind: ParamKindInfo::Roster,
                default: None,
            }
        );
    }

    #[test]
    fn each_workflow_reports_its_execution_mode() {
        let info = inspect(concat!(
            "agent a { prompt #\"x\"# }\n",
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
            "agent b { prompt #\"{{SHARED}}\"# }\n",
        )
        .unwrap();
        let path = dir.path().join("script.gaviero");
        std::fs::write(&path, "include \"lib.gaviero\"\n").unwrap();

        let info = inspect_file(&path).unwrap();
        assert_eq!(info.vars[0].name, "SHARED");
    }
}
