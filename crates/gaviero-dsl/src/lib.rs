pub mod ast;
pub mod compiler;
pub mod error;
mod for_loops;
pub mod inspect;
pub mod lexer;
pub mod parser;
pub mod patterns;
pub mod profile;
pub mod resolver;
pub mod reviewers;
pub mod workflow_params;

// Re-export `CompiledPlan` as the primary output type.
pub use compiler::peek_workflow_execution_mode;
pub use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode};
// Backward-compat alias — existing code using `CompiledScript` keeps compiling.
#[allow(deprecated)]
pub use compiler::CompiledScript;
pub use error::{DslError, DslErrors};
pub use ast::VarKind;
pub use profile::TierOverride;
pub use inspect::{
    AgentInspection, ParamInspection, ParamKindInfo, ProfileClient, ProfileInspection,
    ScriptInspection, VarInspection, WorkflowInspection, inspect_file, inspect_profile,
    profile_tier_names,
};

use gaviero_core::script_profiles::SelectedProfile;

/// Compile an inline `.gaviero` script into a [`CompiledPlan`].
///
/// # Parameters
/// - `source`: The raw script text.
/// - `filename`: Used in error messages to identify the file.
/// - `profile`: The model profile that says which model each tier means.
///   Every agent declares a tier, so a script with agents needs one.
/// - `workflow`: Optional workflow name to select from the script.
///   - `None` and exactly one workflow declared → use it.
///   - `None` and no workflow declared → run all agents in declaration order.
///   - `None` and multiple workflows → error; specify one.
/// - `runtime_prompt`: Optional string substituted for every `{{PROMPT}}` placeholder
///   found in agent `prompt` or `description` fields. If an agent has no `prompt` field
///   and `runtime_prompt` is `Some(_)`, the runtime prompt is used as the agent's full
///   instructions.
///
/// # Errors
/// Returns a [`miette::Report`] with colorful source diagnostics if lexing,
/// parsing, or compilation fails.
pub fn compile(
    source: &str,
    filename: &str,
    profile: Option<&SelectedProfile>,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report> {
    compile_with_vars(source, filename, profile, workflow, runtime_prompt, &[], &[], &[])
}

/// Like [`compile`] but accepts variable / tier / param overrides.
///
/// - `override_vars` — `--var KEY=VALUE` overrides. Beat script-level `vars {}`,
///   lose to agent-level vars.
/// - `agent_tiers` — `--tier AGENT=TIER` overrides. Beat the selected
///   workflow's `tiers { }` block, which beats the tier the agent declares.
/// - `override_params` — `--param NAME=VALUE` list-param overrides:
///   `id=tier,...`, bare ids `a,b`, or a DSL list literal `[{id "a" …}]`.
///   Required params without an in-script default fail compilation when
///   absent here.
#[allow(clippy::too_many_arguments)]
pub fn compile_with_vars(
    source: &str,
    filename: &str,
    profile: Option<&SelectedProfile>,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report> {
    use miette::NamedSource;

    // Phase 1: Lex
    let (tokens, lex_errors) = lexer::lex(source);
    if !lex_errors.is_empty() {
        let errors = lex_errors
            .into_iter()
            .map(|span| DslError::Lex {
                src: NamedSource::new(filename, source.to_string()),
                span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
            })
            .collect::<Vec<_>>();
        return Err(miette::Report::new(DslErrors::new(errors)));
    }

    // Phase 2: Parse
    let (ast, parse_errors) = parser::parse(&tokens, source, filename);
    if !parse_errors.is_empty() {
        return Err(miette::Report::new(DslErrors::new(parse_errors)));
    }
    let Some(mut script) = ast else {
        return Err(miette::Report::msg("parsing produced no output"));
    };
    let mut sources = vec![(filename.to_string(), source.to_string())];
    profile::reject_model_tables(&script, &sources).map_err(miette::Report::new)?;
    if let Some(profile) = profile {
        profile::apply_profile(&mut script, &mut sources, profile, &[])
            .map_err(miette::Report::new)?;
    }

    // Phase 3: Compile
    compiler::compile_ast_with_sources(
        &script,
        &sources,
        workflow,
        runtime_prompt,
        override_vars,
        agent_tiers,
        override_params,
    )
    .map_err(miette::Report::new)
}

/// Compile a `.gaviero` script from disk, resolving any `include "..."`
/// statements transitively. This is the entry point you want for scripts
/// that share `prompt {}` / `agent {}` / `vars {}` across files.
///
/// `entry_path` is the top-level script. `include` paths are resolved
/// relative to the directory of the file containing the `include`. Cycles
/// are rejected with a diagnostic anchored at the offending statement.
///
/// For inline scripts that don't need includes, use [`compile`] /
/// [`compile_with_vars`] — those paths reject `include` statements with a
/// diagnostic pointing the caller here.
pub fn compile_file(
    entry_path: &std::path::Path,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report> {
    compile_file_with_profile(
        entry_path,
        None,
        workflow,
        runtime_prompt,
        override_vars,
        agent_tiers,
        override_params,
    )
}

/// Like [`compile_file`], with the tiers bound by a model profile. Pick the
/// profile with [`gaviero_core::script_profiles::ProfileCatalog::select`].
///
/// A model is chosen in one way only: the profile says what each tier is,
/// and an agent's tier is `agent_tiers` (`--tier`) > the workflow's
/// `tiers { }` block > the agent's own `tier`. Scripts declare no clients.
pub fn compile_file_with_profile(
    entry_path: &std::path::Path,
    profile: Option<&SelectedProfile>,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report> {
    compile_file_with_models(
        entry_path,
        profile,
        &[],
        workflow,
        runtime_prompt,
        override_vars,
        agent_tiers,
        override_params,
    )
}

/// Like [`compile_file_with_profile`], with some of the profile's tiers
/// changed for this run only: pointed at other clients of the same profile
/// (`--tier-client TIER=CLIENT`) and/or run at another effort
/// (`--tier-effort TIER=EFFORT`). A tier the profile does not bind is added
/// when it is given a client. Models are still declared only by the profile.
#[allow(clippy::too_many_arguments)]
pub fn compile_file_with_models(
    entry_path: &std::path::Path,
    profile: Option<&SelectedProfile>,
    tier_overrides: &[TierOverride],
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report> {
    let (mut script, mut sources) = resolver::resolve(entry_path).map_err(miette::Report::new)?;
    profile::reject_model_tables(&script, &sources).map_err(miette::Report::new)?;
    match profile {
        Some(profile) => {
            profile::apply_profile(&mut script, &mut sources, profile, tier_overrides)
                .map_err(miette::Report::new)?;
        }
        None if !tier_overrides.is_empty() => {
            return Err(miette::Report::msg(
                "--tier-client / --tier-effort need a model profile: no profile is selected",
            ));
        }
        None => {}
    }
    compiler::compile_ast_with_sources(
        &script,
        &sources,
        workflow,
        runtime_prompt,
        override_vars,
        agent_tiers,
        override_params,
    )
    .map_err(miette::Report::new)
}

/// Resolve `execution repo|document` for a script on disk (includes resolved).
pub fn workflow_execution_mode(
    entry_path: &std::path::Path,
    workflow: Option<&str>,
) -> Result<ExecutionMode, miette::Report> {
    let (script, _) = resolver::resolve(entry_path).map_err(miette::Report::new)?;
    Ok(compiler::peek_workflow_execution_mode(&script, workflow))
}
