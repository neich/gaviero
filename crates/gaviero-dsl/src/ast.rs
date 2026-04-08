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
    pub memory: Option<MemoryBlock>,
    pub span: Span,
}

/// The `scope { ... }` block inside an agent declaration.
#[derive(Debug, Clone)]
pub struct ScopeBlock {
    pub owned: Vec<String>,
    pub read_only: Vec<String>,
    pub span: Span,
}

/// The `memory { ... }` block inside an agent or workflow declaration.
///
/// Controls per-agent memory namespace routing, importance weighting, and
/// automatic staleness invalidation before the agent runs.
///
/// ```text
/// memory {
///     read_ns            ["prior-audits" "shared"]
///     write_ns           "scan-findings"
///     importance         0.9
///     staleness_sources  ["src/"]
/// }
/// ```
#[derive(Debug, Clone)]
pub struct MemoryBlock {
    /// Namespaces to search when building this agent's memory context.
    /// Additive-merged with any workflow-level `read_ns`.
    pub read_ns: Vec<String>,
    /// Namespace to write this agent's results into.
    /// Overrides any workflow-level `write_ns`.
    pub write_ns: Option<String>,
    /// Retrieval importance weight for memories written by this agent (0.0–1.0).
    /// Agent-only; no workflow-level default. `None` → store default (0.5).
    pub importance: Option<f32>,
    /// Relative paths (from workspace root) whose file hashes are checked for
    /// staleness before this agent runs. Agent-only.
    pub staleness_sources: Vec<String>,
    /// Custom semantic search query. When `Some`, replaces the agent's
    /// `description` as the memory search query. Supports `{{PROMPT}}`.
    pub read_query: Option<(String, Span)>,
    /// Custom search result limit. Overrides the default of 5.
    pub read_limit: Option<(usize, Span)>,
    /// Template for the content written to memory after agent completes.
    /// Supports `{{SUMMARY}}`, `{{FILES}}`, `{{AGENT}}`, `{{DESCRIPTION}}`.
    pub write_content: Option<(String, Span)>,
    pub span: Span,
}

// ── iteration literals ────────────────────────────────────────────────────

/// Parsed strategy literal in a `workflow` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyLit {
    SinglePass,
    Refine,
    /// `best_of_N` where N is parsed from an identifier like `best_of_3`.
    BestOfN(u32),
}

/// The `verify { ... }` block inside a `workflow` declaration.
#[derive(Debug, Clone)]
pub struct VerifyBlock {
    pub compile: bool,
    pub clippy: bool,
    pub test: bool,
    pub span: Span,
}

// ── loop / step items ────────────────────────────────────────────────────

/// A single step in a workflow's `steps [...]` list.
#[derive(Debug, Clone)]
pub enum StepItem {
    /// A reference to a named agent.
    Agent(String, Span),
    /// An explicit loop over a sequence of agents.
    Loop(LoopBlock),
}

/// Exit condition for a `loop` block.
#[derive(Debug, Clone)]
pub enum UntilCondition {
    /// Reuses the verify grammar: `{ compile true test true clippy false }`.
    Verify(VerifyBlock),
    /// A named agent that acts as a judge — returns pass/fail.
    Agent(String, Span),
    /// A shell command — exit code 0 means condition met.
    Command(String, Span),
}

/// The `loop { ... }` block inside a workflow's `steps` list.
///
/// ```text
/// loop {
///     agents [implement verify]
///     max_iterations 5
///     until { compile true test true }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct LoopBlock {
    /// Agents to execute each iteration, in order.
    pub agents: Vec<(String, Span)>,
    /// Termination condition.
    pub until: UntilCondition,
    /// Hard upper bound on iterations.
    pub max_iterations: u32,
    pub span: Span,
}

// ── workflow ──────────────────────────────────────────────────────────────

/// Declares an optional execution plan (ordered steps, concurrency cap).
///
/// ```text
/// workflow feature_dev {
///     steps [
///         researcher
///         loop {
///             agents [implementer verifier]
///             max_iterations 5
///             until { compile true test true }
///         }
///     ]
///     max_parallel 2
///     strategy refine
/// }
/// ```
#[derive(Debug, Clone)]
pub struct WorkflowDecl {
    pub name: String,
    pub name_span: Span,
    pub steps: Option<(Vec<StepItem>, Span)>,
    pub max_parallel: Option<(usize, Span)>,
    pub memory: Option<MemoryBlock>,
    pub strategy: Option<(StrategyLit, Span)>,
    pub test_first: Option<(bool, Span)>,
    pub max_retries: Option<(u32, Span)>,
    pub attempts: Option<(u32, Span)>,
    pub escalate_after: Option<(u32, Span)>,
    pub verify: Option<VerifyBlock>,
    pub span: Span,
}

// ── value literals ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierLit {
    // New canonical values
    Cheap,
    Expensive,
    // Deprecated aliases (kept for backward compat — map to Cheap/Expensive in compiler)
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
