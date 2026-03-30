use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;

/// Top-level parsed script.
#[derive(Debug, Clone)]
pub struct Script {
    pub items: Vec<Item>,
}

/// A top-level declaration in a `.gaviero` file.
#[derive(Debug, Clone)]
pub enum Item {
    Client(ClientDecl),
    Agent(AgentDecl),
    Workflow(WorkflowDecl),
}

// ── client ────────────────────────────────────────────────────────────────

/// Declares an LLM backend configuration.
///
/// ```text
/// client claude_opus {
///     tier coordinator
///     model "claude-opus-4-6"
///     privacy public
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ClientDecl {
    pub name: String,
    pub name_span: Span,
    pub tier: Option<(TierLit, Span)>,
    pub model: Option<(String, Span)>,
    pub privacy: Option<(PrivacyLit, Span)>,
    pub span: Span,
}

// ── agent ─────────────────────────────────────────────────────────────────

/// Declares a work unit (one agent task).
///
/// ```text
/// agent researcher {
///     description "Research the codebase"
///     client claude_opus
///     scope { owned ["docs/"] read_only ["src/**"] }
///     prompt #" ... "#
///     depends_on [other_agent]
///     max_retries 2
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AgentDecl {
    pub name: String,
    pub name_span: Span,
    pub description: Option<(String, Span)>,
    /// References a `ClientDecl` by name.
    pub client: Option<(String, Span)>,
    pub scope: Option<ScopeBlock>,
    pub depends_on: Option<(Vec<(String, Span)>, Span)>,
    pub prompt: Option<(String, Span)>,
    pub max_retries: Option<(u8, Span)>,
    pub span: Span,
}

/// The `scope { ... }` block inside an agent declaration.
#[derive(Debug, Clone)]
pub struct ScopeBlock {
    pub owned: Vec<String>,
    pub read_only: Vec<String>,
    pub span: Span,
}

// ── workflow ──────────────────────────────────────────────────────────────

/// Declares an optional execution plan (ordered steps, concurrency cap).
///
/// ```text
/// workflow feature_dev {
///     steps [researcher, implementer]
///     max_parallel 2
/// }
/// ```
#[derive(Debug, Clone)]
pub struct WorkflowDecl {
    pub name: String,
    pub name_span: Span,
    pub steps: Option<(Vec<(String, Span)>, Span)>,
    pub max_parallel: Option<(usize, Span)>,
    pub span: Span,
}

// ── value literals ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierLit {
    Coordinator,
    Reasoning,
    Execution,
    Mechanical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyLit {
    Public,
    LocalOnly,
}
