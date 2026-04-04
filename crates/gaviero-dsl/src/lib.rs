pub mod ast;
pub mod compiler;
pub mod error;
pub mod lexer;
pub mod parser;

// Re-export `CompiledPlan` as the primary output type.
pub use gaviero_core::swarm::plan::CompiledPlan;
// Backward-compat alias — existing code using `CompiledScript` keeps compiling.
#[allow(deprecated)]
pub use compiler::CompiledScript;
pub use error::{DslError, DslErrors};

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
    compiler::compile_ast(&ast, source, filename, workflow, runtime_prompt)
        .map_err(|e| miette::Report::new(e))
}
