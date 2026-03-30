use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

/// A single DSL error with source location.
#[derive(Debug, Error, Diagnostic)]
pub enum DslError {
    /// Lexer found a character that matches no token.
    #[error("unexpected character")]
    #[diagnostic(code(gaviero::dsl::lex), help("check for unsupported symbols"))]
    Lex {
        #[source_code]
        src: NamedSource<String>,
        #[label("unexpected character here")]
        span: SourceSpan,
    },

    /// Chumsky parser failed to match grammar.
    #[error("syntax error: {reason}")]
    #[diagnostic(code(gaviero::dsl::parse))]
    Parse {
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        span: SourceSpan,
        reason: String,
    },

    /// Semantic error detected during compilation to WorkUnit.
    #[error("{reason}")]
    #[diagnostic(code(gaviero::dsl::compile))]
    Compile {
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        span: SourceSpan,
        reason: String,
    },
}

/// Container for one or more DSL errors.
#[derive(Debug, Error, Diagnostic)]
#[error("{} DSL error(s)", errors.len())]
pub struct DslErrors {
    #[related]
    pub errors: Vec<DslError>,
}

impl DslErrors {
    pub fn new(errors: Vec<DslError>) -> Self {
        Self { errors }
    }

    pub fn single(e: DslError) -> Self {
        Self { errors: vec![e] }
    }
}
