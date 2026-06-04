pub mod ast;
pub mod compiler;
pub mod reviewers;
pub mod workflow_params;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod resolver;
pub mod tiers;

// Re-export `CompiledPlan` as the primary output type.
pub use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode};
pub use compiler::peek_workflow_execution_mode;
// Backward-compat alias — existing code using `CompiledScript` keeps compiling.
#[allow(deprecated)]
pub use compiler::CompiledScript;
pub use error::{DslError, DslErrors};
pub use tiers::load_tier_overrides;

/// Compile a `.gaviero` DSL script into a [`CompiledPlan`].
///
/// # Parameters
/// - `source`: The raw script text.
/// - `filename`: Used in error messages to identify the file.
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
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report> {
    compile_with_vars(source, filename, workflow, runtime_prompt, &[], &[], &[])
}

/// Like [`compile`] but accepts variable / tier / param overrides.
///
/// - `override_vars` — `--var KEY=VALUE` overrides. Beat script-level `vars {}`,
///   lose to agent-level vars.
/// - `override_tiers` — `--tiers-file` bindings. Replace top-level `tier`
///   declarations.
/// - `override_params` — `--param NAME=VALUE` workflow-parameter overrides.
///   Roster params: `id=provider:model[@effort],...`. Client params:
///   `provider:model[@effort]`. Required params without an in-script default
///   fail compilation when absent here.
pub fn compile_with_vars(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    override_tiers: &[(String, String)],
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
    let ast = match ast {
        Some(a) => a,
        None => return Err(miette::Report::msg("parsing produced no output")),
    };

    // Phase 3: Compile
    compiler::compile_ast_with_vars(
        &ast,
        source,
        filename,
        workflow,
        runtime_prompt,
        override_vars,
        override_tiers,
        override_params,
    )
    .map_err(|e| miette::Report::new(e))
}

/// Compile a `.gaviero` script from disk, resolving any `include "..."`
/// statements transitively. This is the entry point you want for scripts
/// that share `client {}` / `prompt {}` / `vars {}` across files.
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
    override_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report> {
    let (script, sources) = resolver::resolve(entry_path).map_err(miette::Report::new)?;
    compiler::compile_ast_with_sources(
        &script,
        &sources,
        workflow,
        runtime_prompt,
        override_vars,
        override_tiers,
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
