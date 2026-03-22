# Gaviero — Four-Tier Model Routing Architecture

> Implementation plan for prompt-level agent orchestration with a coordinator model
> and three execution tiers. The fourth tier (local LLM) is optional.
> Includes the complete verification pipeline specification.
>
> **Companion documents:** This plan extends ARCHITECTURE.md and MEMORY.md.
> Where those documents describe existing behaviour, this plan references them
> rather than restating. Implementers should read all three.
>
> **Notation:** "(→ ARCHITECTURE §N)" and "(→ MEMORY §N)" are cross-references
> to canonical descriptions in the companion documents. Do not duplicate those
> sections — follow the reference.

---

## 1. Design Overview

The current Gaviero swarm architecture treats all agents as peers using the same
backend. This plan introduces a **coordinator pattern** where a single Opus call
decomposes user prompts into a DAG of subtasks, each annotated with a complexity
tier, and the SwarmPipeline dispatches them to the appropriate model. A
multi-strategy verification pipeline validates the aggregate result before
returning control to the user.

```
User Prompt
    │
    ▼
┌──────────────────────────────────────────────┐
│         Coordinator (Opus)                    │
│  • Full repo context + memory enrichment     │
│  • Produces: TaskDAG with tier annotations   │
│  • Selects verification strategy             │
│  • Single call per prompt                    │
└──────────────┬───────────────────────────────┘
               │ TaskDAG
               ▼
┌──────────────────────────────────────────────┐
│         SwarmPipeline (enhanced)              │
│  Routes WorkUnits by tier:                   │
│                                              │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐      │
│  │ Tier 1  │  │ Tier 2  │  │ Tier 3  │      │
│  │ Sonnet  │  │ Haiku   │  │ Local   │      │
│  │         │  │         │  │ (opt.)  │      │
│  └─────────┘  └─────────┘  └─────────┘      │
│                                              │
│  Dependency ordering preserved across tiers  │
└──────────────┬───────────────────────────────┘
               │ Results + diffs
               ▼
┌──────────────────────────────────────────────┐
│         Verification Pipeline (Phase 4)       │
│                                              │
│  Step 1: Structural (tree-sitter AST)        │
│    ↓ pass                                    │
│  Step 2: LLM Diff Review (Sonnet)            │
│    ↓ pass                                    │
│  Step 3: Test Suite Execution                │
│                                              │
│  Early termination on failure at each step.  │
│  Escalation: failed unit → next tier up.     │
└──────────────────────────────────────────────┘
```

### Tier Definitions

| Tier | Model | Role | Typical Tasks | Context Budget |
|------|-------|------|---------------|----------------|
| 0 (Coordinator) | Opus | Plan, decompose, select verification | One call per prompt | Full repo (30–80K tokens) |
| 1 (Reasoning) | Sonnet | Multi-file semantic changes + diff review | Refactor patterns, interface redesign, complex logic | 8–32K tokens |
| 2 (Execution) | Haiku | Single-file focused changes | Extract function, add error handling, write tests | 4–16K tokens |
| 3 (Mechanical) | Local 7B | Rote application of instructions | Renames, call-site updates, import fixes, formatting | 2–8K tokens |

Tier 3 is gated behind a configuration flag. When disabled, Tier 2 absorbs
its workload.

### Dual-Mode Operation

The tiered architecture coexists with the existing single-agent mode. The
`model` field on `WorkUnit` controls which mode applies:

| Scenario | `tier` | `model` | Behaviour |
|----------|--------|---------|-----------|
| Coordinated swarm | Set by coordinator | `None` | `TierRouter` resolves model from `tier` |
| Single-agent prompt | Ignored | `Some("sonnet")` | `model` overrides tier routing entirely |
| Per-unit override | Set by coordinator | `Some("opus")` | `model` wins — allows coordinator to pin a specific model |

**Rule:** When `model` is `Some(_)`, the `TierRouter` is bypassed for that
unit. The `WorkUnit` is dispatched directly to the specified model via its
`backend` field. This preserves backward compatibility with the existing
single-agent flow (TUI chat, non-coordinated `/swarm` commands) and enables
escape hatches for units that need specific models.

**Constraint:** `model` overrides cannot violate privacy. If a unit has
`PrivacyLevel::LocalOnly`, setting `model: Some("sonnet")` is an error —
the pipeline rejects it during Phase 1 validation.

### AcpSession Lifecycle Model

Gaviero currently spawns Claude Code as a one-shot subprocess (`claude --print`)
for every message. There is no persistent session — the process writes to
stdin, closes it, reads the NDJSON response from stdout, and the child exits.
This means slash commands like `/compact` have no Claude-side session to act on.

The tiered architecture introduces a **dual-mode AcpSession** that supports
both one-shot and persistent subprocess lifecycles. The persistent mode spawns
`claude` (without `--print`) in conversation mode, keeps the child alive across
turns, and writes subsequent prompts to stdin without closing it. Message
boundaries are detected by `ResultEvent` in the NDJSON stream.

```rust
/// Session lifecycle mode for AcpSession
pub enum SessionMode {
    /// Spawn `claude --print`, write stdin, close, read stdout, child exits.
    /// Deterministic cost. Clean isolation. No state carryover.
    OneShot,
    /// Spawn `claude` (no --print), keep alive, bidirectional stdin/stdout.
    /// Slash commands (e.g. /compact) forwarded as raw stdin lines.
    /// Context accumulates across turns within the session.
    Persistent,
}
```

**Which callers use which mode:**

| Caller | Session Mode | Rationale |
|--------|-------------|-----------|
| TUI chat agent | Persistent | User converses across many messages; `/compact` useful |
| Coordinator (Phase 1) | OneShot | One call per swarm run; predictable cost |
| Coordinator (Phase 4 opt.) | Persistent | Retains project understanding across runs (→ §9 Phase 4) |
| Swarm execution agents | OneShot | Independent per-WorkUnit; no state to carry over |
| DiffReviewer | OneShot | One batch → one JSON verdict → done |
| MergeResolver | OneShot | Resolve conflicts per-merge, no carryover |

**Persistent sessions — subprocess protocol:**

```
TUI/Coordinator                       claude (persistent child)
     │                                      │
     │  write prompt to stdin (no close)    │
     │ ──────────────────────────────────►  │
     │                                      │  processes, streams NDJSON
     │  ◄──────────────────────────────────  │
     │        ...ContentDelta...            │
     │        ...ToolUseStart...            │
     │        ResultEvent (end-of-turn)     │
     │                                      │
     │  write next prompt / /compact        │
     │ ──────────────────────────────────►  │
     │                                      │
     │  (repeat until session dropped)      │
     │                                      │
     │  drop AcpSession → kill child        │
     └──────────────────────────────────────┘
```

**NDJSON parsing changes:** The existing `StreamEvent` enum and NDJSON parser
remain unchanged. The only difference is lifecycle: in OneShot mode, EOF on
stdout signals session end. In Persistent mode, `ResultEvent` signals
end-of-turn (ready for next prompt), and EOF signals the child died unexpectedly.

**Slash command forwarding:** In Persistent mode, when the user types a Claude
Code slash command (e.g., `/compact`, `/model sonnet`), the TUI writes the
raw command text to stdin as a new "turn". Claude Code processes it internally
and the session continues. The NDJSON stream may or may not produce output
depending on the command.

### AcpSessionFactory — `gaviero-core/src/acp/session.rs`

```rust
pub struct AcpSessionFactory {
    default_tools: Vec<Tool>,
    agent_options: AgentOptions,
}

impl AcpSessionFactory {
    /// Create a one-shot session: spawn `claude --print`, write prompt,
    /// close stdin, read NDJSON to completion.
    /// Used by: swarm agents, DiffReviewer, MergeResolver, Coordinator (Phase 1).
    pub fn one_shot(
        &self,
        model: &str,
        tools: &[Tool],
        write_mode: WriteMode,
    ) -> Result<AcpSession>;

    /// Create or retrieve a persistent session: spawn `claude` (no --print),
    /// keep alive, bidirectional stdin/stdout.
    /// Used by: TUI chat, Coordinator (Phase 4 optional).
    ///
    /// The `key` parameter identifies the session purpose (e.g., "chat",
    /// "coordinator:{workspace_hash}"). If a session with that key is already
    /// alive, returns a reference to it. If it died or was never created,
    /// spawns a new one.
    pub fn persistent(
        &self,
        key: &str,
        model: &str,
        tools: &[Tool],
        write_mode: WriteMode,
    ) -> Result<AcpSession>;

    /// Send /compact to a persistent session and store evicted context
    /// in memory before compaction. This preserves critical information
    /// that would otherwise be lost when Claude Code truncates its context.
    ///
    /// Flow:
    /// 1. Query the session for a summary of its current context (if possible)
    /// 2. Store summary in memory with key "session:{key}:compact:{timestamp}"
    /// 3. Write "/compact" to stdin
    /// 4. Wait for ResultEvent confirming compaction
    pub async fn compact_with_memory_backup(
        &self,
        key: &str,
        memory: &MemoryStore,
        namespace: &str,
    ) -> Result<()>;

    /// Check if a persistent session is alive and responsive.
    pub fn is_session_alive(&self, key: &str) -> bool;

    /// Kill a persistent session. Called on workspace close or explicit reset.
    pub fn kill_session(&self, key: &str);
}
```

**Session lifecycle management:**

- Persistent sessions are keyed by purpose string (e.g., `"chat"`,
  `"coordinator:a1b2c3"`). The factory owns the child process handles.
- On workspace close or TUI quit, all persistent sessions are killed via
  `Drop` on the factory.
- If a persistent session's child dies unexpectedly (EOF on stdout), the
  factory detects this on the next `send_prompt()` call and returns an error.
  The caller can then request a new session via the same key.
- `compact_with_memory_backup()` is the bridge between session context
  management and the memory system. It prevents information loss during
  compaction by storing a summary before `/compact` runs.

---

## 2. Changes to Existing Modules

### 2.1 New Types — `gaviero-core/src/types.rs`

```rust
/// Model tier for task routing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelTier {
    /// Coordinator: planning, decomposition, verification (Opus)
    Coordinator,
    /// Complex multi-file semantic reasoning (Sonnet)
    Reasoning,
    /// Focused single-file execution (Haiku)
    Execution,
    /// Mechanical rote changes — optional, local LLM (Qwen 2.5 Coder 7B)
    Mechanical,
}

/// Privacy classification for routing decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyLevel {
    /// Can be sent to any API-based model
    Public,
    /// Must stay on local model only
    LocalOnly,
}

/// Coordinator-produced task with tier annotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierAnnotation {
    pub tier: ModelTier,
    pub privacy: PrivacyLevel,
    pub estimated_context_tokens: u32,
    pub rationale: String,
}
```

### 2.2 Extended WorkUnit — `gaviero-core/src/swarm/models.rs`

The existing `WorkUnit` gains tier routing metadata:

```rust
pub struct WorkUnit {
    pub id: String,
    pub description: String,
    pub scope: FileScope,
    pub depends_on: Vec<String>,
    pub backend: AgentBackend,
    // --- EXISTING FIELD — REINTERPRETED ---
    /// Per-unit model override. When `Some(_)`, bypasses `TierRouter` for
    /// this unit — the agent runs on the specified model directly.
    /// Used for: single-agent prompts, coordinator escape hatches.
    /// When `None`, the `TierRouter` resolves the model from `tier`.
    /// Privacy constraint: setting `model` to an API model on a
    /// `LocalOnly` unit is a validation error (caught in Phase 1).
    pub model: Option<String>,
    // --- NEW FIELDS ---
    pub tier: ModelTier,                    // Assigned by coordinator
    pub privacy: PrivacyLevel,             // Routing constraint
    pub coordinator_instructions: String,   // Opus's decomposed instructions
    pub estimated_tokens: u32,             // Context budget hint
    pub max_retries: u8,                   // Before escalation (default: 1)
    pub escalation_tier: Option<ModelTier>, // Tier to escalate to on failure
    pub format_version: u8,                // Schema version for stored entries (current: 1)
}
```

### 2.3 Extended AgentBackend — `gaviero-core/src/swarm/models.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentBackend {
    ClaudeCode,
    Codex,
    GeminiCli,
    Ollama { model: String, base_url: String },
    Custom(String),
}
```

> **Note for ARCHITECTURE.md update:** The `GeminiCli` variant is part of the
> provider-agnostic agent integration layer. ARCHITECTURE.md §3 should list
> `GeminiCli` alongside `ClaudeCode` and `Codex` in the `AgentBackend` enum.

### 2.4 New Configuration Keys — `settings.json`

```json
{
    "agent": {
        "coordinator": {
            "model": "opus",
            "maxContextTokens": 80000,
            "persistent": false,
            "compactThreshold": 60000
        },
        "tiers": {
            "reasoning": { "model": "sonnet", "maxParallel": 3 },
            "execution": { "model": "haiku", "maxParallel": 6 },
            "mechanical": {
                "enabled": false,
                "backend": "ollama",
                "ollamaModel": "qwen2.5-coder:7b",
                "ollamaBaseUrl": "http://localhost:11434",
                "maxParallel": 8,
                "maxContextTokens": 8192
            }
        },
        "routing": {
            "privacyPatterns": ["**/clinical/**", "**/grading/**"],
            "escalationEnabled": true,
            "costBudget": null
        },
        "memory": {
            "enrichCoordinator": true,
            "enrichAgents": true,
            "enrichReviewer": true,
            "coordinatorMemoryLimit": 10,
            "agentMemoryLimit": 5,
            "reviewerMemoryLimit": 5,
            "storeVerificationSummaries": true,
            "storeTierAccuracy": true,
            "crossRunContinuity": true,
            "retentionMaxRuns": 50,
            "retentionMaxAgeDays": 30
        },
        "verification": {
            "defaultStrategy": "combined",
            "structural": {
                "checkMissingSymbols": true,
                "errorNodeThreshold": 0
            },
            "diffReview": {
                "model": "sonnet",
                "batchStrategy": "perDependencyTier",
                "maxDiffTokens": 16384,
                "reviewTiers": ["mechanical", "execution"]
            },
            "testSuite": {
                "command": null,
                "timeout": 300,
                "targeted": true,
                "autoDetect": true,
                "inheritEnv": ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME",
                               "GOPATH", "JAVA_HOME", "NODE_PATH", "PYTHONPATH",
                               "VIRTUAL_ENV", "CONDA_PREFIX", "DATABASE_URL",
                               "RUST_LOG", "RUST_BACKTRACE", "CI"]
            }
        }
    }
}
```

---

## 3. New Modules

### 3.1 Coordinator — `gaviero-core/src/swarm/coordinator.rs`

This is the central new component. It replaces the current `planner.rs` with a
tier-aware planning system.

**Responsibilities:**

1. Receive the user prompt + full repo context + memory enrichment
2. Query memory for prior run results to detect cross-run continuity opportunities
3. Build a memory-aware repo context (memory summaries substitute for raw files)
4. Call Opus once to produce a `TaskDAG`
5. Annotate each node with `ModelTier`, `PrivacyLevel`, dependency edges
6. Select a `VerificationStrategy` based on task characteristics
7. Validate the DAG (scope overlaps, cycles, tier feasibility, filesystem existence)
8. Return the DAG to SwarmPipeline for dispatch

**Failure handling:** If the Opus call fails (network error, rate limit, timeout),
the coordinator returns `Err` immediately. The SwarmPipeline surfaces this to the
user via `SwarmObserver::on_swarm_completed` with a clear error. No fallback to
degraded planning — the coordinator is a hard dependency of the tiered pipeline.
For single-agent, non-coordinated usage, the existing `AcpPipeline` path remains
available and does not depend on the coordinator.

If the coordinator is using a persistent session (Phase 4 opt-in) and the child
process dies, the factory detects this on the next `plan()` call, spawns a fresh
session automatically, and retries once. The new session starts cold (no prior
context), but memory enrichment (→ §10.1) compensates.

**Session mode:** In Phase 1, the coordinator uses `SessionMode::OneShot` — one
Opus subprocess per swarm run, predictable cost, clean isolation. In Phase 4,
an optional `coordinator.persistent: true` setting enables
`SessionMode::Persistent` — the coordinator keeps an Opus session alive across
swarm runs within the same editor session. See §9 Phase 4 for implications.

```rust
pub struct Coordinator {
    session_factory: AcpSessionFactory,
    memory: Option<Arc<MemoryStore>>,
    config: CoordinatorConfig,
    /// When persistent=true, the coordinator reuses this session key
    /// across plan() calls. The factory manages the child process lifetime.
    session_key: Option<String>,  // e.g., "coordinator:{workspace_hash}"
}

pub struct TaskDAG {
    pub plan_summary: String,
    pub units: Vec<WorkUnit>,         // Already tier-annotated
    pub dependency_graph: Vec<(String, String)>,  // (from, to) edges
    pub verification_strategy: VerificationStrategy,
    pub continued_from: Option<String>, // Prior run ID if cross-run continuation
}

impl Coordinator {
    /// Single Opus call: prompt → TaskDAG
    ///
    /// Memory integration (→ §10.1):
    /// 1. search_context_filtered() with ExcludeLocalOnly for API-safe enrichment
    /// 2. Prior verification summaries inform tier assignment calibration
    /// 3. Prior agent results enable cross-run continuity (skip completed work)
    /// 4. Memory summaries reduce raw file context needed (→ §10.2)
    pub async fn plan(
        &self,
        prompt: &str,
        repo_context: &RepoContext,
        memory_context: &str,
    ) -> Result<TaskDAG>;

    /// Check memory for a recent failed/partial run matching this prompt.
    /// Returns completed unit IDs that can be skipped in the new plan.
    ///
    /// Matching uses two strategies:
    /// 1. Explicit: if the prompt contains "run:{run_id}", look up that run directly
    /// 2. Semantic: search memory for agent results matching the prompt (score > 0.8)
    ///
    /// Explicit matching is preferred when available — it avoids the fragility
    /// of semantic similarity on rephrased prompts.
    async fn detect_continuity(
        &self,
        prompt: &str,
        namespaces: &[String],
    ) -> Option<ContinuityContext>;

    /// Validate coordinator output beyond JSON parsing:
    /// - All depends_on references point to valid unit IDs in the DAG
    /// - All owned_paths exist in the workspace filesystem
    /// - estimated_tokens ≤ tier's maxContextTokens budget
    /// - No LocalOnly unit has model override pointing to API backend
    /// - No scope overlap (delegates to existing validate_scopes())
    fn validate_dag(&self, dag: &TaskDAG, workspace: &Workspace) -> Result<()>;
}

pub struct ContinuityContext {
    pub prior_run_id: String,
    pub completed_units: Vec<String>,   // Unit IDs that succeeded
    pub failed_units: Vec<FailedUnit>,  // Units that need re-planning
    pub prior_plan_summary: String,
    pub prior_dependency_graph: Vec<(String, String)>, // For cascading dependency analysis
}

pub struct FailedUnit {
    pub id: String,
    pub failure_reason: String,
    pub tier_at_failure: ModelTier,
}
```

### 3.2 Verification Strategy — `gaviero-core/src/swarm/verify/mod.rs`

The coordinator selects a strategy during planning. The strategy determines which
verification steps run in Phase 4.

```rust
pub enum VerificationStrategy {
    /// Tree-sitter structural validation only.
    /// Cheapest. For pure mechanical changes and speed-critical runs.
    StructuralOnly,

    /// Sonnet reviews diffs from specified tiers.
    /// For behavioral changes without a test suite.
    DiffReview {
        review_tiers: Vec<ModelTier>,
        batch_strategy: BatchStrategy,
    },

    /// Run test suite after merge.
    /// For projects with tests where structural + tests suffice.
    TestSuite {
        command: String,
        targeted: bool,
    },

    /// All three strategies in sequence with early termination.
    /// Default for non-trivial swarm executions.
    Combined {
        review_tiers: Vec<ModelTier>,
        test_command: Option<String>,
    },
}

pub enum BatchStrategy {
    /// One Sonnet call per WorkUnit — most thorough, most expensive
    PerUnit,
    /// Group all units sharing a dependency tier into one review call
    PerDependencyTier,
    /// Single review call for all diffs — cheapest, risks context overflow
    Aggregate,
}
```

**Coordinator override vs user settings:** If the coordinator's strategy
conflicts with user settings (e.g., coordinator says "structural only" but user
configured `defaultStrategy: "combined"`), the **more thorough strategy wins**.
The coordinator can reduce review scope (e.g., "only review mechanical tier")
but cannot disable a verification step the user has explicitly enabled.

### 3.3 Structural Verifier — `gaviero-core/src/swarm/verify/structural.rs`

Catches syntactic and structural damage — broken ASTs, orphaned symbols,
malformed imports, unbalanced brackets. Zero LLM calls, zero subprocess spawns,
runs in milliseconds. Answers: "Did the agents produce syntactically valid code?"

**When the coordinator should select this alone:**
- All subtasks are mechanical (renames, import updates, formatting)
- The project has no test suite or tests are not relevant to the change
- Speed is critical (interactive single-file edits, rapid iteration)
- Always runs as the first step inside `Combined`

```rust
pub struct StructuralVerifier;

pub struct StructuralReport {
    pub files_checked: usize,
    pub files_passed: usize,
    pub failures: Vec<StructuralFailure>,
}

pub struct StructuralFailure {
    pub path: PathBuf,
    pub language: String,
    pub error_nodes: Vec<ErrorNode>,
    pub severity: FailureSeverity,
}

pub struct ErrorNode {
    pub line: usize,
    pub column: usize,
    pub byte_range: std::ops::Range<usize>,
    pub parent_symbol: Option<String>,  // enclosing function/struct name
    pub context_snippet: String,         // 3 lines around the error
}

pub enum FailureSeverity {
    /// Tree-sitter ERROR node — definite parse failure
    ParseError,
    /// MISSING node — tree-sitter recovered but something is absent
    MissingNode,
    /// Symbol referenced in coordinator instructions but absent in final AST
    MissingSymbol { expected: String },
}
```

**Prerequisite refactoring in `tree_sitter.rs`:**

The structural verifier needs the AST parent-walking logic currently embedded
inside `enrich_hunks()`. Before implementing this module, extract a shared
helper:

```rust
// gaviero-core/src/tree_sitter.rs — new public function
/// Walk up from `node` to find the nearest enclosing named definition
/// (function, class, struct, enum, trait, impl, method).
/// Used by: enrich_hunks() (existing), StructuralVerifier (new).
pub fn find_enclosing_symbol(node: Node) -> Option<NodeInfo>
```

Then refactor `enrich_hunks()` to call `find_enclosing_symbol()` instead of
inlining the walk. This is the only change to `tree_sitter.rs`.

**Data flow:**

```
StructuralVerifier::verify(dag, modified_files)
  │
  │  For each file modified by any agent in the DAG:
  │
  ├─ 1. Read final file content from disk (post-merge)
  │
  ├─ 2. Resolve language via language_for_extension()
  │     │
  │     ├─ Language found → tree-sitter parse
  │     └─ Unknown extension → skip (not a failure)
  │
  ├─ 3. Parse with tree-sitter, walk AST for ERROR and MISSING nodes
  │     │
  │     │  For each ERROR/MISSING node:
  │     │    - Record line, column, byte range
  │     │    - find_enclosing_symbol(node) → parent context
  │     │    - Extract 3-line context snippet
  │     │
  │     └─ Collect into Vec<ErrorNode>
  │
  ├─ 4. Symbol presence check (optional, per-unit)
  │     │
  │     │  If the WorkUnit's coordinator_instructions mention specific
  │     │  symbol names (e.g., "create function validate_token"), verify
  │     │  the symbol exists in the final AST.
  │     │
  │     │  Implementation: tree-sitter tag query for function/class/struct
  │     │  definitions, match against expected names extracted from
  │     │  coordinator_instructions via regex.
  │     │
  │     └─ Missing expected symbols → MissingSymbol failure
  │
  └─ 5. Aggregate into StructuralReport

Return: StructuralReport
  - files_passed == files_checked → verification passed
  - Any failures → report to caller with full context
```

**Cost:** Zero. No LLM calls, no network, no subprocess. Pure CPU.
Latency: <100ms for typical refactors (tens of files).

**Limitations:**
- Cannot detect semantic errors (logic bugs, wrong variable, off-by-one)
- Cannot detect behavioral regressions
- Cannot verify the change implements the intended behavior
- Does not catch errors in languages without tree-sitter grammars (graceful skip)
- ERROR nodes have false positives in some grammars (e.g., template literals
  in JS/TS occasionally produce spurious ERROR nodes in tree-sitter)

**Failure handling:**

```
StructuralReport has failures
  │
  ├─ Identify which WorkUnit(s) touched the failing files
  │   (from AgentManifest.modified_files)
  │
  ├─ For each failing unit:
  │   ├─ ParseError severity → trigger escalation (→ §7)
  │   │
  │   └─ MissingSymbol severity → trigger escalation (→ §7)
  │
  └─ MissingNode severity → log warning, do not escalate
     (tree-sitter MISSING nodes are often false positives from error recovery)
```

### 3.4 LLM Diff Reviewer — `gaviero-core/src/swarm/verify/diff_review.rs`

Catches semantic errors that structural verification misses — wrong logic,
incomplete changes, misinterpreted instructions, subtle side effects. Answers:
"Did the agents correctly implement what the coordinator intended?"

**When the coordinator should select this:**
- Any subtasks were assigned to Mechanical tier (local 7B)
- The change involves behavioral modifications (not just renames)
- The coordinator decomposed a complex task into many small subtasks
- High-stakes changes (auth, payment, data models)
- No test suite available to validate against

**Design principle — Review model > Execution model:**

| Code produced by | Reviewed by | Rationale |
|-------------------|-------------|-----------|
| Mechanical (local 7B) | Sonnet | Two tiers up — catches both rote and semantic errors |
| Execution (Haiku) | Sonnet | One tier up — catches semantic misunderstandings |
| Reasoning (Sonnet) | — | Not reviewed by default (would need Opus, too expensive) |

The coordinator can override this and request Opus review of Sonnet output for
critical subtasks, but this is opt-in and expensive.

```rust
pub struct DiffReviewer {
    session_factory: AcpSessionFactory,
    config: DiffReviewConfig,
}
```

**Session mode:** Always `SessionMode::OneShot`. Each review batch spawns a
fresh `claude --print` subprocess with `WriteMode::RejectAll`. No state carries
between batches — each review call is independent.

pub struct DiffReviewConfig {
    pub review_model: String,        // Default: "sonnet"
    pub max_diff_tokens: u32,        // Truncation limit per review call
    pub batch_strategy: BatchStrategy,
    pub review_tiers: Vec<ModelTier>, // Which tiers get reviewed
}

pub struct DiffReviewReport {
    pub reviews: Vec<UnitReview>,
    pub aggregate_approved: bool,
}

pub struct UnitReview {
    pub unit_id: String,
    pub approved: bool,
    pub issues: Vec<ReviewIssue>,
}

pub struct ReviewIssue {
    pub severity: IssueSeverity,
    pub file: PathBuf,
    pub line_range: Option<(usize, usize)>,
    pub description: String,
    pub suggested_fix: Option<String>,
}

pub enum IssueSeverity {
    /// Blocks approval — must be fixed
    Error,
    /// Flagged but doesn't block — logged for user review
    Warning,
}
```

**Data flow:**

```
DiffReviewer::review(dag, manifests, memory, batch_strategy)
  │
  ├─ 1. Filter: select WorkUnits whose tier ∈ review_tiers
  │     Skip units that weren't executed (failed, blocked)
  │
  ├─ 2. Compute diffs for each selected unit:
  │     │
  │     │  For each file in manifest.modified_files:
  │     │    original = git2 blob content at pre-merge commit
  │     │    current  = fs::read_to_string(path) (post-merge content)
  │     │    diff     = diff_engine::compute_hunks(original, current)
  │     │    enriched = tree_sitter::enrich_hunks(diff, original, language)
  │     │
  │     │  IMPORTANT — Attribution accuracy: when two agents modified adjacent
  │     │  code and the merge interleaved their changes, the diff for one
  │     │  unit's files may include merge-introduced changes. To mitigate:
  │     │  compute diffs against the agent's own worktree branch merge-base,
  │     │  NOT against HEAD~1. This isolates each agent's actual contribution.
  │     │
  │     │    merge_base = git2::Repository::merge_base(agent_branch, main_pre_merge)
  │     │    original = git2 blob content at merge_base
  │     │
  │     └─ Produces Vec<(WorkUnit, Vec<StructuralHunk>)>
  │
  ├─ 2b. Memory enrichment for review context (→ §10.6):
  │     │
  │     │  memory.search_context_filtered(
  │     │    namespaces, dag.plan_summary, reviewerMemoryLimit, ExcludeLocalOnly)
  │     │
  │     │  If memory is None, this step is skipped (graceful degradation).
  │     │  Memory entries count against the maxDiffTokens budget. If adding
  │     │  memory would push over the limit, memory entries are truncated
  │     │  first (diffs are essential; memory is supplementary).
  │     │
  │     └─ Appended as Section 2 of the review prompt (→ §5.3)
  │
  ├─ 3. Batch according to strategy:
  │     │
  │     │  PerUnit:
  │     │    Each (WorkUnit, hunks) → one Sonnet call
  │     │    Parallel, bounded by reasoning Semaphore
  │     │
  │     │  PerDependencyTier:
  │     │    Group units by their dependency tier index
  │     │    Each group → one Sonnet call with all diffs concatenated
  │     │
  │     │  Aggregate:
  │     │    All units → one Sonnet call
  │     │    Risk: may exceed context limit → fallback to PerDependencyTier
  │     │
  │     └─ For each batch, assemble review prompt
  │
  ├─ 4. Execute review calls:
  │     │
  │     │  For each batch:
  │     │    Spawn AcpSession(model: sonnet, WriteMode::RejectAll)
  │     │    Send review prompt → stream NDJSON
  │     │    Parse response for JSON verdict: { approved, issues[] }
  │     │
  │     └─ Collect into Vec<UnitReview>
  │
  └─ 5. Aggregate results
       aggregate_approved = all individual reviews approved
       Return DiffReviewReport
```

**Token budget management:**

```
Total diff tokens estimated (rough: 4 chars/token)
  │
  ├─ Under max_diff_tokens → proceed with selected batch strategy
  │
  ├─ Over limit, Aggregate strategy → fall back to PerDependencyTier
  │
  ├─ Over limit, PerDependencyTier → fall back to PerUnit
  │
  └─ Single unit over limit → truncate to changed hunks only,
     drop unchanged context lines, include a note:
     "[Diff truncated — showing changed regions only]"
```

**Cost:**
- PerUnit batching: ~$0.003-0.01 per reviewed unit (Sonnet, 4-16K context)
- PerDependencyTier: ~$0.005-0.02 per dependency tier
- Aggregate: ~$0.01-0.03 for the whole swarm run

**Failure handling:**

```
DiffReviewReport has rejected units
  │
  ├─ For each rejected unit with Error-severity issues:
  │   Build repair prompt with reviewer's issues + suggested fixes
  │   Escalation follows the protocol in §7
  │
  └─ Warning-severity issues:
     Logged in SwarmResult.verification_warnings
     Displayed in swarm dashboard for user awareness
     Do NOT trigger re-execution
```

### 3.5 Test Runner — `gaviero-core/src/swarm/verify/test_runner.rs`

Catches behavioral regressions — code that parses correctly and looks right to a
reviewer but doesn't actually work. Answers: "Does the codebase still pass its
own tests after the changes?"

**When the coordinator should select this:**
- The project has a test suite the coordinator can identify
- The changes involve behavioral modifications (not just renames or formatting)
- The changes touch code that has corresponding test coverage
- The user has configured a test command in settings

```rust
pub struct TestRunner {
    config: TestRunnerConfig,
}

pub struct TestRunnerConfig {
    /// Command to execute (e.g., "cargo test", "npm test", "pytest")
    pub command: String,
    /// Working directory (defaults to workspace root)
    pub cwd: Option<PathBuf>,
    /// Timeout in seconds (default: 300 = 5 minutes)
    pub timeout_secs: u64,
    /// Environment variables to explicitly inherit from the parent process.
    /// Configurable via agent.verification.testSuite.inheritEnv.
    /// Defaults: PATH, HOME, CARGO_HOME, RUSTUP_HOME, GOPATH, JAVA_HOME,
    ///   NODE_PATH, PYTHONPATH, VIRTUAL_ENV, CONDA_PREFIX, DATABASE_URL,
    ///   RUST_LOG, RUST_BACKTRACE, CI
    pub inherit_env: Vec<String>,
    /// Additional environment variables to set
    pub extra_env: HashMap<String, String>,
    /// Whether to run only tests related to modified files
    pub targeted: bool,
}

pub struct TestReport {
    pub exit_code: i32,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub targeted_filter: Option<String>,
    pub parsed_results: Option<ParsedTestResults>,
}

pub struct ParsedTestResults {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub failures: Vec<TestFailure>,
}

pub struct TestFailure {
    pub test_name: String,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}
```

**Data flow:**

```
TestRunner::run(dag, manifests, workspace_root)
  │
  ├─ 1. Resolve test command:
  │     │
  │     │  Priority order:
  │     │  a. dag.verification_strategy.test_command (coordinator-specified)
  │     │  b. settings.agent.verification.testSuite.command (user-configured)
  │     │  c. Auto-detect from project files:
  │     │     Cargo.toml → "cargo test"
  │     │     package.json with "test" script → "npm test"
  │     │     pytest.ini / setup.cfg / pyproject.toml → "pytest"
  │     │     build.gradle / build.gradle.kts → "gradle test"
  │     │     Makefile with "test" target → "make test"
  │     │  d. None found → return TestReport { passed: true, note: "no test suite" }
  │     │
  │     └─ Store resolved command
  │
  ├─ 2. Build targeted test filter (if config.targeted = true):
  │     │
  │     │  Collect all modified files from manifests
  │     │
  │     │  For Rust (cargo test):
  │     │    Extract module paths from file paths
  │     │    e.g., src/auth/strategy.rs → "auth::strategy"
  │     │    Filter: "cargo test auth::strategy auth::oauth auth::basic"
  │     │
  │     │  For JavaScript/TypeScript (jest):
  │     │    Match test files by convention:
  │     │    src/auth.ts → tests/auth.test.ts, __tests__/auth.test.ts
  │     │    Filter: "npx jest --testPathPattern='auth|oauth|basic'"
  │     │
  │     │  For Python (pytest):
  │     │    Match test files: src/auth.py → tests/test_auth.py
  │     │    Filter: "pytest tests/test_auth.py tests/test_oauth.py"
  │     │
  │     │  For Java (gradle/maven):
  │     │    Match test classes: src/main/java/auth/Strategy.java →
  │     │    "--tests 'auth.StrategyTest'"
  │     │
  │     │  If no matching test files found → fall back to full suite
  │     │
  │     └─ Store targeted_filter
  │
  ├─ 3. Execute test command:
  │     │
  │     │  Build environment:
  │     │    - Start with empty env (no implicit leakage)
  │     │    - For each key in config.inherit_env:
  │     │        if std::env::var(key) exists → include it
  │     │    - Apply config.extra_env overrides on top
  │     │
  │     │  std::process::Command::new("sh")
  │     │    .arg("-c")
  │     │    .arg(&resolved_command)
  │     │    .current_dir(&workspace_root)
  │     │    .env_clear()
  │     │    .envs(&inherited)
  │     │    .envs(&config.extra_env)
  │     │    .stdout(Stdio::piped())
  │     │    .stderr(Stdio::piped())
  │     │    .spawn()
  │     │
  │     │  Note: this is the only subprocess spawn in the swarm pipeline.
  │     │  All git operations use git2 (→ ARCHITECTURE §17 Hard Constraint #2).
  │     │  Test execution requires a subprocess by nature.
  │     │
  │     │  Timeout: tokio::time::timeout(Duration::from_secs(timeout_secs))
  │     │    On timeout → kill child, return TestReport { passed: false,
  │     │      stderr: "Test execution timed out after {timeout}s" }
  │     │
  │     └─ Capture stdout, stderr, exit code
  │
  ├─ 4. Parse test output (best-effort):
  │     │
  │     │  Rust: regex "test result: (ok|FAILED). (\d+) passed; (\d+) failed"
  │     │  Jest: JSON reporter output or regex "Tests: X failed, Y passed"
  │     │  Pytest: regex "(\d+) passed, (\d+) failed"
  │     │
  │     │  Parse failure → ParsedTestResults remains None
  │     │  The raw stdout/stderr is always available regardless
  │     │
  │     └─ Store ParsedTestResults if parsing succeeded
  │
  └─ 5. Return TestReport

Return: TestReport
  - passed = (exit_code == 0)
  - All raw output preserved for user inspection
  - Parsed failures available for targeted escalation
```

**Tests run on the merged codebase** (post Phase 3), not individual worktrees.
Worktrees are cleaned up in Phase 3. If a test needs worktree-specific setup
(e.g., test fixtures created by an agent), that setup must survive the merge.

**Cost:** Zero LLM cost. Subprocess time only.

**Failure handling:** Test failures trigger escalation per §7. Attribution uses
`TestFailure.file` → `WorkUnit.scope.owned_paths` matching. Unattributed
failures (no file path, no scope match) are logged in `SwarmResult` for user
inspection without triggering escalation.

### 3.6 Combined Verification — `gaviero-core/src/swarm/verify/combined.rs`

Applies all three strategies in sequence with early termination. Default for
non-trivial swarm executions.

**Execution order** (cheapest first, early termination on failure):

```
Combined { review_tiers, test_command }
  │
  ├─ Step 1: STRUCTURAL VERIFICATION (always runs)
  │   │
  │   │  StructuralVerifier::verify(dag, modified_files)
  │   │  Cost: ~0ms, $0.00
  │   │
  │   ├─ Failures with ParseError severity?
  │   │   YES → escalate failing units (→ §7), then restart Step 1
  │   │   NO  ↓
  │   │
  │   └─ MissingSymbol warnings → log, continue
  │
  ├─ Step 2: DIFF REVIEW (if review_tiers is non-empty)
  │   │
  │   │  DiffReviewer::review(dag, manifests, batch_strategy)
  │   │  Only reviews units whose tier ∈ review_tiers
  │   │  Cost: ~$0.005-0.03, ~10-30s
  │   │
  │   ├─ Any units rejected with Error severity?
  │   │   YES → escalate failing units (→ §7)
  │   │         Re-run Step 1 on re-executed files only
  │   │         Do NOT re-run diff review (→ §7 loop prevention)
  │   │   NO  ↓
  │   │
  │   └─ Warning-severity issues → log, continue
  │
  └─ Step 3: TEST SUITE (if test_command is Some)
      │
      │  TestRunner::run(dag, manifests, workspace_root)
      │  Cost: $0.00, ~5-120s subprocess time
      │
      ├─ Tests pass?
      │   YES → verification complete, all passed
      │   NO  → attribute failures, escalate if possible (→ §7)
      │
      └─ Return CombinedReport
```

**Early termination logic:**

- Structural failures (Step 1) block everything — no point reviewing code that
  doesn't parse.
- Diff review rejections (Step 2) trigger escalation before test execution — if
  the reviewer says the logic is wrong, running tests wastes time.
- Test failures (Step 3) are the final gate — the hardest class of error and the
  most valuable to catch.

```rust
pub struct CombinedReport {
    pub structural: StructuralReport,
    pub diff_review: Option<DiffReviewReport>,
    pub test_suite: Option<TestReport>,
    pub overall_passed: bool,
    pub escalations_performed: Vec<EscalationRecord>,
    pub cost_estimate: CostEstimate,
}

pub struct EscalationRecord {
    pub unit_id: String,
    pub reason: EscalationReason,
    pub from_tier: ModelTier,
    pub to_tier: ModelTier,
    pub succeeded: bool,
}

pub enum EscalationReason {
    StructuralParseError,
    DiffReviewRejection { issues: Vec<ReviewIssue> },
    TestFailure { test_names: Vec<String> },
}

pub struct CostEstimate {
    pub coordinator_tokens: u64,
    pub reasoning_tokens: u64,
    pub execution_tokens: u64,
    pub mechanical_tokens: u64,
    pub review_tokens: u64,
    pub estimated_usd: f64,
}
```

### 3.7 Ollama Backend — `gaviero-core/src/swarm/ollama.rs`

A new agent backend for local LLM execution via the Ollama HTTP API.

```rust
pub struct OllamaBackend {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub async fn generate(
        &self,
        prompt: &str,
        system: &str,
        context_limit: u32,
    ) -> Result<String>;

    /// Health check — returns false if Ollama is unreachable
    pub async fn is_available(&self) -> bool;
}
```

This module talks to Ollama's `/api/generate` endpoint with streaming disabled.
The response is parsed for `<file>` blocks identically to ACP session output,
reusing `AcpPipeline::extract_file_blocks()`.

**Design choice:** The Ollama backend does NOT use the ACP subprocess model.
It makes direct HTTP calls. This avoids requiring Claude Code CLI for local
model execution and keeps the dependency surface minimal.

### 3.8 Tier Router — `gaviero-core/src/swarm/router.rs`

Maps `ModelTier` + `PrivacyLevel` to concrete backend configuration, respecting
the `model` override field.

```rust
pub struct TierRouter {
    config: TierConfig,
    ollama_available: bool,  // Cached health check result
}

pub enum ResolvedBackend {
    Claude { model: String },
    Ollama { model: String, base_url: String },
    Blocked { reason: String },
}

impl TierRouter {
    /// Resolve a WorkUnit to a concrete backend + model string.
    ///
    /// Resolution order:
    /// 1. If unit.model is Some → use it directly (privacy-checked in Phase 1)
    /// 2. Else → route by (tier, privacy, ollama_available)
    pub fn resolve(&self, unit: &WorkUnit) -> ResolvedBackend {
        // model override takes precedence
        if let Some(ref model) = unit.model {
            return self.resolve_model_override(unit, model);
        }

        match (unit.tier, unit.privacy, self.ollama_available) {
            // Privacy-sensitive: force local regardless of tier
            (_, PrivacyLevel::LocalOnly, true) => ResolvedBackend::Ollama { .. },
            (_, PrivacyLevel::LocalOnly, false) => {
                ResolvedBackend::Blocked { reason: "local model required but unavailable".into() }
            }

            // Normal routing
            (ModelTier::Coordinator, _, _) => ResolvedBackend::Claude { model: "opus".into() },
            (ModelTier::Reasoning, _, _) => ResolvedBackend::Claude { model: "sonnet".into() },
            (ModelTier::Execution, _, _) => ResolvedBackend::Claude { model: "haiku".into() },

            // Mechanical: local if available and enabled, else fall back to Haiku
            (ModelTier::Mechanical, _, true) if self.config.mechanical.enabled =>
                ResolvedBackend::Ollama { .. },
            (ModelTier::Mechanical, _, _) =>
                ResolvedBackend::Claude { model: "haiku".into() },
        }
    }

    /// Handle escalation after subtask failure
    pub fn escalate(&self, unit: &WorkUnit) -> Option<ResolvedBackend> {
        match unit.escalation_tier {
            Some(tier) => Some(self.resolve_tier(tier, unit.privacy)),
            None => None,
        }
    }
}
```

### 3.9 Privacy Scanner — `gaviero-core/src/swarm/privacy.rs`

Evaluates `PrivacyLevel` for files based on configured glob patterns.

```rust
pub struct PrivacyScanner {
    patterns: Vec<glob::Pattern>,
}

/// Filter for memory search operations
#[derive(Debug, Clone, Copy)]
pub enum PrivacyFilter {
    /// Return all entries — for local-only agents
    IncludeAll,
    /// Exclude entries with privacy: LocalOnly — for API-bound context
    ExcludeLocalOnly,
}

impl PrivacyScanner {
    /// Check if any file in a WorkUnit's scope matches privacy patterns
    pub fn classify(&self, scope: &FileScope) -> PrivacyLevel;
}
```

The coordinator also suggests privacy levels, but the scanner overrides to
`LocalOnly` if any owned or read-only path matches a configured pattern.
This is a safety net — the privacy decision is never purely LLM-determined.

---

## 4. Modified Modules

### 4.1 SwarmPipeline — `gaviero-core/src/swarm/pipeline.rs`

The existing three-phase pipeline (Validate → Execute → Merge) gains a
Phase 0 (Plan) and Phase 4 (Verify).

```
Phase 0: PLAN (NEW)
  │  Memory enrichment (→ §10.1):
  │    search_context_filtered(namespaces, prompt, limit, ExcludeLocalOnly)
  │  Cross-run continuity detection (→ §10.3):
  │    Coordinator::detect_continuity(prompt, namespaces)
  │  Memory-aware repo context building (→ §10.2):
  │    RepoContextBuilder uses memory summaries to substitute for raw files
  │  Coordinator::plan(prompt, repo_context, memory) → TaskDAG
  │  Coordinator::validate_dag() → semantic validation of DAG output
  │  PrivacyScanner::classify() overrides on all units
  │  TierRouter health check (Ollama ping)
  │  Cost estimate → if costBudget set and estimate exceeds it, abort with report
  │
Phase 1: VALIDATE (existing, unchanged)
  │  validate_scopes(), dependency_tiers()
  │  NEW: reject units where model override violates privacy
  │
Phase 2: EXECUTE (modified)
  │  For each dependency tier:
  │    Group WorkUnits by ModelTier
  │    Dispatch each group to appropriate backend via TierRouter
  │    Per-model-tier Semaphores bound concurrency independently
  │    Collect AgentManifests
  │    On failure: attempt escalation via TierRouter::escalate()
  │
  │  Dependency tier completion rule: a dependency tier is "complete" when
  │  ALL units in that tier have finished (success, failure, or escalation
  │  resolution). No unit from dependency tier N+1 starts until tier N is
  │  complete. The per-model-tier Semaphores control concurrency WITHIN a
  │  dependency tier — they do not allow cross-tier execution.
  │
Phase 3: MERGE (existing, conflict resolution escalated to Opus)
  │  MergeResolver now uses Opus instead of default model
  │
Phase 4: VERIFY (NEW)
     Execute VerificationStrategy from TaskDAG (→ §3.2–3.6)
     On failure: escalate → re-execute → re-merge → re-verify (→ §7)
     Store results in MemoryStore (→ §10.5 for privacy protocol)
     Return SwarmResult with CombinedReport + CostEstimate
```

**Per-model-tier Semaphores:**

```rust
struct TierSemaphores {
    reasoning: Arc<Semaphore>,   // config.tiers.reasoning.maxParallel
    execution: Arc<Semaphore>,   // config.tiers.execution.maxParallel
    mechanical: Arc<Semaphore>,  // config.tiers.mechanical.maxParallel
}
```

These prevent cheap Haiku/local calls from being blocked by slow Sonnet calls
within the same dependency tier. They operate strictly within a dependency tier —
they do not allow units from different dependency tiers to interleave.

### 4.2 AgentRunner — `gaviero-core/src/swarm/runner.rs`

Extended to dispatch to the appropriate backend based on `ResolvedBackend`.

**Session mode:** All swarm execution agents use `SessionMode::OneShot`
(`claude --print`). Each WorkUnit gets its own independent subprocess in its
own worktree. No state carries between agents — this is by design, since agents
work in isolated branches with disjoint file scopes. A persistent session would
leak context between unrelated units and complicate worktree cleanup.

```rust
impl AgentRunner {
    pub async fn run(&self, unit: &WorkUnit, resolved: &ResolvedBackend) -> Result<AgentManifest> {
        match resolved {
            ResolvedBackend::Claude { model } => self.run_acp(unit, model).await,
            ResolvedBackend::Ollama { model, base_url } => self.run_ollama(unit, model, base_url).await,
            ResolvedBackend::Blocked { reason } => Err(anyhow!("Blocked: {reason}")),
        }
    }

    async fn run_ollama(&self, unit: &WorkUnit, model: &str, base_url: &str) -> Result<AgentManifest> {
        // 1. Build focused prompt: coordinator_instructions + file contents from scope
        // 2. Memory enrichment (→ §10.4 — critical for mechanical tier)
        // 3. Call OllamaBackend::generate()
        // 4. Extract <file> blocks from response
        // 5. Route each through propose_write() (WriteMode::AutoAccept)
        // 6. Return AgentManifest
    }
}
```

### 4.3 TaskPlanner — `gaviero-core/src/swarm/planner.rs`

The existing `TaskPlanner` becomes a thin wrapper around `Coordinator` for
backward compatibility. The old `plan_task()` method delegates to
`Coordinator::plan()` and flattens the result to `Vec<WorkUnit>`.

### 4.4 Observer Events — `gaviero-core/src/observer.rs`

New SwarmObserver callbacks:

```rust
pub trait SwarmObserver: Send + Sync {
    // existing callbacks...

    // Coordination lifecycle
    fn on_coordination_started(&self, prompt: &str);
    fn on_coordination_complete(&self, dag: &TaskDAG);
    fn on_tier_dispatch(&self, unit_id: &str, tier: ModelTier, backend: &str);
    fn on_escalation(&self, unit_id: &str, from_tier: ModelTier, to_tier: ModelTier);

    // Verification lifecycle
    fn on_verification_started(&self, strategy: &VerificationStrategy);
    fn on_verification_step_started(&self, step: VerificationStep);
    fn on_verification_step_complete(&self, step: VerificationStep, passed: bool);
    fn on_verification_escalation(&self, record: &EscalationRecord);
    fn on_verification_complete(&self, report: &CombinedReport);

    // Cost tracking
    fn on_cost_update(&self, estimate: &CostEstimate);
}

pub enum VerificationStep {
    Structural,
    DiffReview { units_to_review: usize },
    TestSuite { command: String, targeted: bool },
}
```

### 4.5 Swarm Dashboard — `gaviero-tui/src/panels/swarm_dashboard.rs`

Enhanced to show tier-level and verification information:

- Color-coded tier badges per agent row (Opus=purple, Sonnet=blue, Haiku=green, Local=yellow)
- Coordination phase indicator (planning → executing → verifying)
- Running cost estimate (updated via `on_cost_update`)
- Escalation events highlighted in the log
- Backend indicator column (Claude API / Ollama)
- Three-stage verification progress indicator:

```
Verification: [✓ Structural] [● Diff Review] [○ Tests]
              3/3 files OK    reviewing 2u    waiting
```

Each stage transitions: `○` (pending) → `●` (running) → `✓` (passed)
or `✗` (failed). Escalation events appear as inline log entries below.

---

## 5. Prompt Engineering

### 5.1 Coordinator System Prompt (Opus)

```
You are a code architect decomposing a development task into parallelizable
subtasks. For each subtask, assign:

- tier: "reasoning" | "execution" | "mechanical"
  - reasoning: multi-file semantic changes, interface redesigns, complex logic
  - execution: single-file focused changes, test writing, error handling
  - mechanical: renames, import updates, call-site propagation, formatting

- privacy: "public" | "local_only"
  - local_only: if the file path matches privacy patterns or contains
    sensitive domain data

- depends_on: IDs of subtasks that must complete first
- scope: files this subtask owns (write) and reads (read-only)
- estimated_tokens: approximate context needed (file contents + instructions)

MEMORY CONTEXT (if provided):
The [Memory context] section contains knowledge from previous runs on this
project. Use it to:
- Calibrate tier assignments: if memory shows mechanical tasks on certain
  file types frequently escalated, assign those to execution tier instead.
- Avoid redundant work: if memory shows a prior run already completed
  certain changes, do not include those as subtasks.
- Reference established patterns: memory may describe interfaces, naming
  conventions, and architectural decisions. Reference these in your
  coordinator_instructions so execution agents can apply them without
  re-deriving them.

CROSS-RUN CONTINUITY (if prior_completed_units provided):
A previous run partially completed this task. The following units succeeded
and their changes are already merged: {completed_unit_ids}. Plan ONLY the
remaining work. You may reference the completed units in depends_on if the
new subtasks build on their output.

Select a verification_strategy:
- "structural_only": when ALL subtasks are mechanical (renames, formatting)
- "diff_review": when there are behavioral changes but no test suite.
    Specify review_tiers (which tiers to review) and batch_strategy.
- "test_suite": when a test suite exists and structural + tests suffice.
    Specify the test command and whether to use targeted mode.
- "combined": (DEFAULT) when mixing tiers, complex refactors, or uncertain.
    Specify review_tiers and optional test_command.

Output a JSON object with:
{ plan_summary, units: [...], verification_strategy: { type, ... } }
```

### 5.2 Mechanical Tier Prompt Template

Local model prompts must be maximally constrained to compensate for reduced
reasoning capacity:

```
You are executing a precise code modification task. Follow these instructions
exactly. Do not add, remove, or modify anything beyond what is specified.

TASK: {coordinator_instructions}

FILES YOU MAY MODIFY:
{file_contents_from_scope}

OUTPUT: For each file you modify, output the complete file wrapped in:
<file path="relative/path">
...complete file content...
</file>

RULES:
- Output ONLY <file> blocks. No explanations, no commentary.
- Include the COMPLETE file content, not just changed sections.
- Do NOT rename files, create new files, or delete files.
- Preserve all existing formatting, comments, and whitespace
  except where the task specifically requires changes.
```

### 5.3 Diff Review Prompt (Sonnet)

```
SECTION 1: COORDINATOR CONTEXT
────────────────────────────────
The original task was: "{dag.plan_summary}"

This subtask was assigned to a {tier_name} model with these instructions:
"{unit.coordinator_instructions}"

SECTION 2: SUPPLEMENTARY CONTEXT (from project memory)
────────────────────────────────
{memory_context — omitted if memory is None or empty}

SECTION 3: DIFFS
────────────────────────────────
{for each file in unit's modified files}

--- a/{path}
+++ b/{path}

{unified diff with structural annotations}

  In function `{enclosing_symbol}`:
    - Line {start}: {hunk description}

{end for}

SECTION 4: REVIEW INSTRUCTIONS
────────────────────────────────
Evaluate on these axes:

1. CORRECTNESS — Do the changes implement the coordinator's instructions?
2. COMPLETENESS — Are all required changes present?
3. SCOPE DISCIPLINE — Did the agent modify anything outside its instructions?
4. INTERFACE PRESERVATION — Are function signatures / public API preserved
   or correctly updated as instructed?
5. CONSISTENCY — Do changes across files use consistent naming and patterns?

Respond with ONLY a JSON object:
{
  "approved": true/false,
  "issues": [
    {
      "severity": "error" | "warning",
      "file": "path/to/file",
      "line_range": [start, end] or null,
      "description": "what's wrong",
      "suggested_fix": "how to fix it" or null
    }
  ]
}
```

---

## 6. Data Flow: Complete Tiered Execution

```
User: "Refactor the auth module to use the strategy pattern"
  │
  ▼
TUI/CLI: handle_swarm_command()
  │
  ├─ 1. Memory enrichment: memory.search_context_filtered(
  │      namespaces, prompt, 10, ExcludeLocalOnly)
  ├─ 2. Repo context: collect file tree, key interfaces, recent git log
  │
  ▼
Coordinator::plan(prompt, repo_context, memory_context)
  │
  │  [Opus call — ~30s, ~$0.05]
  │  Ingests full repo context (50K tokens)
  │  Returns TaskDAG:
  │
  │  TaskDAG {
  │    plan_summary: "Refactor auth into strategy pattern...",
  │    units: [
  │      { id: "design-interface",     tier: Reasoning, desc: "Define AuthStrategy trait..." },
  │      { id: "impl-oauth",           tier: Execution, desc: "Implement OAuthStrategy..." },
  │      { id: "impl-basic",           tier: Execution, desc: "Implement BasicStrategy..." },
  │      { id: "update-callsites-api", tier: Mechanical, desc: "Replace direct auth calls in api/*.rs..." },
  │      { id: "update-callsites-web", tier: Mechanical, desc: "Replace direct auth calls in web/*.rs..." },
  │      { id: "update-imports",       tier: Mechanical, desc: "Update use statements in 12 files..." },
  │      { id: "write-tests",          tier: Execution, desc: "Add unit tests for AuthStrategy..." },
  │    ],
  │    verification: Combined {
  │      review_tiers: [Mechanical],
  │      test_command: Some("cargo test --lib")
  │    }
  │  }
  │
  ▼
Coordinator::validate_dag() — semantic checks
  │
  ▼
SwarmPipeline::execute(dag)
  │
  ├─ Phase 1: VALIDATE
  │   validate_scopes() ✓
  │   dependency_tiers() → [
  │     Dep Tier 0: [design-interface]              → Sonnet (Reasoning)
  │     Dep Tier 1: [impl-oauth, impl-basic,        → Sonnet×2 + Local×1
  │                   update-imports]
  │     Dep Tier 2: [update-callsites-api,          → Local×2 + Haiku×1
  │                   update-callsites-web,
  │                   write-tests]
  │   ]
  │
  ├─ Phase 2: EXECUTE
  │   Dep Tier 0:
  │     design-interface → Sonnet [~15s, ~$0.01]
  │
  │   Dep Tier 1 (parallel, per-model Semaphores):
  │     impl-oauth       → Sonnet [~20s, ~$0.01]  (reasoning Semaphore)
  │     impl-basic       → Sonnet [~15s, ~$0.01]  (reasoning Semaphore)
  │     update-imports   → Local  [~3s, $0.00]    (mechanical Semaphore)
  │     ── Dep Tier 1 complete when ALL three finish ──
  │
  │   Dep Tier 2 (parallel):
  │     update-callsites-api → Local [~2s, $0.00]
  │     update-callsites-web → Local [~2s, $0.00]
  │     write-tests          → Haiku [~8s, ~$0.003]
  │
  ├─ Phase 3: MERGE
  │   7 branches → main (Opus for conflicts if any)
  │
  └─ Phase 4: VERIFY (Combined strategy)
     │
     ├─ Step 1: Structural verification
     │   Parse all modified files with tree-sitter
     │   Result: 14 files checked, 14 passed [<100ms, $0.00]
     │
     ├─ Step 2: Diff review (Mechanical tier only)
     │   Review 3 units: update-callsites-api, update-callsites-web, update-imports
     │   BatchStrategy: PerDependencyTier (2 Sonnet calls)
     │   Result: all approved [~10s, ~$0.005]
     │
     └─ Step 3: Test suite
        cargo test --lib (targeted: auth::strategy auth::oauth auth::basic)
        Result: 12 tests passed, 0 failed [~8s, $0.00]

Total: ~65s wall clock, ~$0.08
  vs single-model (all Opus): ~180s, ~$0.50
  vs single-model (all Sonnet): ~120s, ~$0.10
```

---

## 7. Escalation Protocol

All escalation follows this single protocol. Other sections reference §7 rather
than restating escalation mechanics.

### 7.1 Global Bounds

Each WorkUnit gets at most **3 total attempts**: 1 original + 1 retry at same
tier + 1 escalation to `escalation_tier`. After 3 attempts, the unit is marked
`Failed` and its dependents are skipped.

Default escalation chain: `Mechanical → Execution → Reasoning → Failed`

The coordinator assigns `escalation_tier` per-unit. Trivially mechanical tasks
(renames) get `escalation_tier: Some(Execution)`. Complex execution tasks get
`escalation_tier: Some(Reasoning)`. Reasoning tasks have no escalation — if
Sonnet fails, it's reported to the user.

### 7.2 Re-Execution via Fresh Worktree (Incremental Merge)

When verification rejects a unit post-merge, the pipeline performs an
**incremental re-merge** rather than reverting the original merge:

```
Escalation triggered for unit U (post Phase 3 merge)
  │
  ├─ 1. Create fresh worktree for U: .gaviero/worktrees/{unit_id}_esc/
  │     Branch from current main (which includes the original merge)
  │
  ├─ 2. Re-execute U at escalation_tier with repair context:
  │     │  The escalated agent sees the CURRENT state of main (including
  │     │  all other agents' merged work). Its repair prompt includes:
  │     │  - Original coordinator_instructions
  │     │  - Specific failure reason (parse error / review issues / test output)
  │     │  - The current file contents (post-merge, including the bad changes)
  │     │  The agent writes corrected files.
  │
  ├─ 3. Incremental merge: merge the escalation branch into main
  │     │  This is a targeted fix on top of the existing merge.
  │     │  Conflicts are unlikely (the escalated agent is fixing its own files)
  │     │  but handled by MergeResolver if they occur.
  │
  ├─ 4. Re-verify (scoped):
  │     │  - Structural: re-check only files touched by the escalated agent
  │     │  - Diff review: NOT re-run (loop prevention — see §7.3)
  │     │  - Tests: re-run only the originally failing tests, not the full suite
  │
  └─ 5. Cleanup worktree (WorktreeManager Drop)
```

**Advantage over revert-and-redo:** The escalated agent works on the current
state of main, which includes all other agents' successfully merged work. This
means it can see and respect changes from parallel agents — it doesn't need to
re-derive context that was already merged.

**Session mode:** The escalated agent uses `SessionMode::OneShot` — a fresh
`claude --print` subprocess, like all swarm agents. It has no memory of the
original agent's session.

**Risk:** If the escalated agent's fix introduces a conflict with another agent's
code, the incremental merge catches it. The MergeResolver handles conflicts
using the same Opus-based resolution as Phase 3.

### 7.3 Loop Prevention

1. The `reviewed_units: HashSet<String>` in `DiffReviewer` prevents re-reviewing
   the same unit. After escalation, only structural verification runs.
2. Test re-runs are scoped to originally failing tests only — preventing the
   "fix one test, break another" loop.
3. If an escalated attempt fails structural verification, the unit is marked
   `Failed` — no further retries.
4. The 3-attempt bound is absolute. No exception.

### 7.4 Escalation by Failure Source

| Source | Repair prompt includes | Re-dispatch to | Post-escalation verification |
|--------|----------------------|----------------|------------------------------|
| Execution error (malformed output) | Error message, original instructions | Same tier (retry), then escalation_tier | Structural only |
| Structural ParseError | Line, column, snippet, enclosing symbol | escalation_tier | Structural only |
| Structural MissingSymbol | Expected symbol name, target file | escalation_tier | Structural only |
| Diff review Error | Reviewer issues + suggested fixes | Same tier (retry), then escalation_tier | Structural only |
| Test failure | Test name, output, task context | escalation_tier | Structural + re-run failing tests |

---

## 8. Privacy Routing

Two mechanisms enforce privacy:

1. **Pattern-based (deterministic):** `routing.privacyPatterns` globs in settings.
   Any file matching these patterns forces `PrivacyLevel::LocalOnly` on any
   WorkUnit whose scope includes that file. The PrivacyScanner runs after the
   coordinator returns the DAG, overriding the coordinator's privacy suggestions.

2. **Coordinator-suggested:** Opus can flag tasks as `local_only` based on
   semantic understanding (e.g., recognizing clinical terminology). This is
   advisory — the PrivacyScanner never downgrades from `LocalOnly` to `Public`.

When a `LocalOnly` task encounters an unavailable local backend:
- The task is `Blocked`, not escalated to an API model
- The SwarmResult includes the blocked task with a clear reason
- The user is informed via `SwarmObserver::on_tier_dispatch` with `Blocked` status

**Interaction with `model` override:** Phase 1 validation rejects any WorkUnit
where `privacy == LocalOnly` and `model` points to an API backend. This prevents
the override mechanism from bypassing privacy constraints.

---

## 9. Implementation Phases

### Phase 1: Foundation (coordinator + two API tiers)

**Goal:** Opus plans, Sonnet and Haiku execute. No local model or verification
pipeline yet. Privacy filtering in memory from day one. AcpSession dual-mode
infrastructure.

1. Add `ModelTier`, `PrivacyLevel`, `TierAnnotation` to `types.rs`
2. Add `SessionMode` enum and `AcpSessionFactory` to `acp/session.rs`:
   - `one_shot()` → `claude --print` (existing behaviour, extracted)
   - `persistent()` → `claude` (no --print), bidirectional stdin/stdout
   - Persistent session lifecycle: child keepalive, EOF detection, auto-respawn
   - NDJSON parsing: `ResultEvent` as end-of-turn delimiter in persistent mode
   - Slash command forwarding: raw stdin write for `/compact`, `/model`, etc.
3. Migrate existing `AcpSession` callers to use `AcpSessionFactory::one_shot()`:
   - `AcpPipeline` (TUI chat) — migrated to `persistent()` with key `"chat"`
   - `AgentRunner` — uses `one_shot()`
   - This is a refactoring step with no behavioural change for swarm agents
4. Add `format_version: u8` to `WorkUnit`
5. Extend `WorkUnit` with tier fields; document `model` override semantics
6. Implement `Coordinator` in `swarm/coordinator.rs`:
   - Uses `factory.one_shot()` for Opus calls (persistent deferred to Phase 4)
   - Opus prompt engineering (including memory context sections)
   - JSON response parsing into `TaskDAG`
   - `validate_dag()` for semantic validation of coordinator output
   - Fallback: if JSON parsing fails, retry once with repair prompt
   - If Opus call fails entirely, return `Err` (no degraded mode)
   - `detect_continuity()` with hybrid matching (explicit run-id + semantic)
7. Implement `TierRouter` in `swarm/router.rs`:
   - `model` override takes precedence over tier routing
   - Privacy validation (LocalOnly + API model = reject)
8. Modify `SwarmPipeline` Phase 2 to use per-tier Semaphores and route
   through `TierRouter`; enforce dependency tier completion boundary
9. Add coordinator + memory + routing config keys to settings cascade
10. Update `TaskPlanner` to delegate to `Coordinator`
11. New SwarmObserver coordination + cost events; TUI dashboard tier badges
12. Schema migration: add `privacy` column to `memories` table (→ §10.5)
13. `PrivacyFilter` enum and `search_context_filtered()` in `MemoryStore`
14. Privacy-aware memory storage: agent results stored with `privacy` column
15. Cost estimator: live cost tracking via `on_cost_update`, optional
    `costBudget` abort

**Testing:** Unit tests for `AcpSessionFactory` (one-shot lifecycle, persistent
session keepalive, EOF recovery, slash command forwarding), `TierRouter::resolve()`
(including model override and privacy rejection), `Coordinator::validate_dag()`,
JSON parsing with mock Opus responses, `detect_continuity()` with both explicit
and semantic matching, `search_context_filtered()` with LocalOnly entries,
integration test with two-tier dispatch.

### Phase 2: Verification pipeline

**Goal:** Post-execution quality assurance with all three strategies.

1. Create `swarm/verify/` module structure
2. Extract `find_enclosing_symbol()` from `tree_sitter.rs` into shared helper
   (prerequisite for structural verifier — see §3.3)
3. Implement `StructuralVerifier`:
   - AST error/missing node walking
   - Symbol presence check against coordinator instructions
4. Implement `DiffReviewer`:
   - Batch strategy selection and fallback chain
   - Review prompt construction (§5.3) with memory supplementary context
   - Diff computation against agent branch merge-base (not HEAD~1) for
     accurate attribution (→ §3.4)
   - JSON verdict parsing with error recovery
   - Token budget management and truncation
   - `WriteMode::RejectAll` on reviewer sessions
5. Implement `TestRunner`:
   - Test command auto-detection
   - Targeted test filtering per language ecosystem
   - Environment inheritance via `inherit_env` config
   - Output parsing into `ParsedTestResults`
   - Timeout handling
6. Implement `CombinedVerification`:
   - Three-step sequential execution with early termination
   - Escalation via fresh worktree + incremental merge (→ §7.2)
   - Loop prevention via `reviewed_units` set (→ §7.3)
7. Wire Phase 4 into `SwarmPipeline`
8. Verification config keys in settings cascade
9. SwarmObserver verification events
10. Dashboard: three-stage progress indicator, escalation log entries

**Testing:**
- Structural: mock tree-sitter parse with injected ERROR nodes, symbol check
- DiffReview: mock AcpSession returning approval/rejection JSON;
  verify merge-base diff computation produces correct attribution
- TestRunner: mock subprocess with known stdout patterns per framework;
  verify `inherit_env` populates correctly
- Combined: orchestration test with structural fail → escalation → recheck
- Loop prevention: verify max 3 attempts per unit

### Phase 3: Local model backend (optional tier)

**Goal:** Ollama integration as the fourth tier.

1. Implement `OllamaBackend` in `swarm/ollama.rs`:
   - HTTP client with `/api/generate` endpoint
   - Health check via `/api/tags`
   - `<file>` block extraction reusing existing parser
2. Implement `PrivacyScanner` in `swarm/privacy.rs`
3. Extend `TierRouter` with Ollama dispatch + `Blocked` handling
4. Add `mechanical` tier config
5. `AgentRunner::run_ollama()` implementation
6. Dashboard: backend indicator column, local model status

**Testing:** Mock Ollama server for unit tests. Integration test with real
Ollama (gated behind `#[cfg(feature = "ollama-tests")]`).

### Phase 4: Refinements

1. **Adaptive tier assignment:** If the coordinator consistently over-assigns
   to Reasoning (Sonnet succeeds on first try), log a suggestion to adjust.
2. **Memory-aware repo context builder (→ §10.2):** `RepoContextBuilder` uses
   memory summaries to substitute for raw file content.
3. **Memory pruning (→ §10.8):** Implement retention policy based on
   `retentionMaxRuns` and `retentionMaxAgeDays`.
4. **Compound improvement tracking:** Dashboard shows per-project memory depth
   and tier accuracy trends over the last N runs.
5. **Persistent coordinator session:** When `coordinator.persistent: true`, the
   coordinator uses `factory.persistent("coordinator:{workspace_hash}", "opus")`
   instead of `factory.one_shot()`. Implications:
   - The Opus session accumulates project understanding across swarm runs within
     the same editor session. It retains context from previous plans without
     relying on memory search.
   - Context window exhaustion: when estimated token usage exceeds
     `coordinator.compactThreshold` (default: 60K), the pipeline calls
     `factory.compact_with_memory_backup()` — storing a summary of the
     session's context in memory before sending `/compact` to Claude Code.
     After compaction, memory enrichment (→ §10.1) backfills critical context.
   - Cost attribution: with a persistent session, Opus cost cannot be cleanly
     attributed to a single swarm run. `CostEstimate` reports "session cost
     delta" (tokens consumed since last plan call) rather than total call cost.
   - Interaction with `RepoContextBuilder` (→ §10.2): a persistent session
     retains prior repo understanding natively. The context builder can send
     only delta context (files changed since last run) instead of the full
     repo. This compounds with memory-based context reduction.
   - Interaction with `detect_continuity()` (→ §10.3): the persistent session
     provides native continuity for same-session retries. Memory-based
     continuity remains necessary for cross-session restarts and after
     compaction events.
   - Session end: on workspace close or TUI quit, the factory kills the
     persistent coordinator session. Next workspace open starts fresh (but
     memory persists across sessions).

---

## 10. Memory Integration

Memory is the connective tissue of the tiered architecture. It enables the
coordinator to improve over time, reduces context window costs, prevents
redundant work across runs, improves lower-tier agent success rates, and
enriches the verification pipeline — all while respecting privacy boundaries.

All memory operations follow the lock discipline defined in MEMORY.md §Lock
Discipline. All memory access is gracefully optional (→ MEMORY.md §Design
Rules item 8). These properties are not restated per-section below.

### 10.1 Coordinator Calibration (Memory → Better Plans)

Before each coordinator call, `search_context_filtered()` retrieves relevant
project knowledge. This context includes three categories:

**Verification summaries** (key pattern: `verification:{run_id}`):

Tell Opus how each tier performed historically. If mechanical-tier tasks on
Rust import files escalated twice in the last three runs, Opus adjusts —
assigning those to Execution tier or writing more constrained instructions.

**Prior agent results** (key pattern: `agents:{run_id}:{unit_id}`):

Describe what previous agents did — which files they modified, what interfaces
they created, what patterns they used. When the current task overlaps with
previous work, Opus references these instead of re-deriving understanding.

**Tier accuracy stats** (key pattern: `tiers:{run_id}`):

Per-tier success rates, average escalation count, average tokens used vs
estimated. The quantitative signal enabling calibration.

```rust
// Stored after each swarm run — key: "tiers:{run_id}"
memory.store(
    namespace,
    &format!("tiers:{run_id}"),
    &format!(
        "Tier accuracy for run {run_id}: \
         mechanical={}/{} (escalations: {}), \
         execution={}/{} (escalations: {}), \
         reasoning={}/{} (escalations: {}). \
         Token estimates vs actual: mech={}/{}, exec={}/{}, reason={}/{}",
        // ... counters
    ),
    Some(&serde_json::to_string(&EntryMetadata {
        privacy: PrivacyLevel::Public,
        format_version: 1,
        source: "swarm_pipeline".into(),
    })?),
).await;
```

**Warm-up consideration:** Early runs produce noisy stats. The coordinator
prompt includes a note to weight recent runs more heavily and distinguish
instruction-quality failures from model-capability failures.

### 10.2 Memory-Aware Context Building (Memory → Cheaper Opus Calls)

The `RepoContextBuilder` (Phase 4 refinement) queries memory before assembling
the coordinator's context:

```
RepoContextBuilder::build(workspace, memory, namespaces)
  │
  ├─ 1. Collect full file tree (paths + sizes)
  │
  ├─ 2. For each module/directory, check memory for recent summaries:
  │     │
  │     │  memory.search(namespace, "module: src/auth/", 3)
  │     │
  │     ├─ Memory entry found AND recent AND relevant:
  │     │   "Recent" = git2 blob hash of described files matches current HEAD.
  │     │   (Uses git2::Repository::blob_path() — NOT filesystem mtime,
  │     │    which is unreliable across checkout/merge/build operations.)
  │     │   Include memory summary instead of raw files.
  │     │
  │     └─ No memory, stale, or blob hash mismatch:
  │        Include raw file content as normal
  │
  ├─ 3. Always include: files directly referenced in the prompt,
  │     files matching tree-sitter interface extraction (pub trait/fn/struct)
  │
  └─ 4. Assemble context with budget tracking:
       Total tokens estimated → trim lowest-relevance entries if over budget
```

**Staleness detection:** Compares git2 blob hashes of described files against
current HEAD. Blob hashes are content-addressed — they change if and only if
file content changes, unlike `mtime` which changes on `git checkout`, merge,
and build operations.

**Impact:** For mature projects (10+ runs), context reduction of 30-40% on
coordinator input.

**Interaction with persistent coordinator session (Phase 4):** If the
coordinator uses `SessionMode::Persistent`, it retains prior repo understanding
natively in its conversation history. The `RepoContextBuilder` then only sends
delta context (files changed since the last `plan()` call), reducing per-run
input further. After a `/compact` event, `RepoContextBuilder` falls back to
full context mode until the session accumulates enough history again. The
`compact_with_memory_backup()` call ensures that pre-compaction knowledge is
preserved in memory, so memory-based context substitution compensates for the
compaction loss.

### 10.3 Cross-Run Continuity (Memory → No Redundant Work)

When a swarm run partially succeeds, `detect_continuity()` enables the next
attempt to skip completed work:

```
Coordinator::detect_continuity(prompt, namespaces)
  │
  ├─ 1a. Explicit match: parse prompt for "run:{run_id}" reference
  │      If found → look up that run's entries directly (exact key match)
  │
  ├─ 1b. Semantic match (fallback if no explicit reference):
  │      memory.search(namespace, prompt, 20)
  │      Filter to runs where plan_summary score > 0.8 AND run age < 72h
  │
  ├─ 2. Group results by run_id (from key pattern "agents:{run_id}:{unit_id}")
  │
  ├─ 3. For the best-matching prior run:
  │     Collect agent entries → partition into succeeded / failed
  │     Load verification entry → extract failure details
  │
  └─ 4. Return ContinuityContext:
       completed_units: units that succeeded AND whose files haven't
         changed since (verified by git2 blob hash comparison)
       failed_units: with failure reason and tier at failure
       prior_plan_summary: for coordinator alignment
       prior_dependency_graph: for cascading dependency analysis
```

**Explicit matching (`/retry run:abc123`)** is preferred when available — it
avoids the fragility of semantic similarity on rephrased prompts.

**Impact:** A failed 7-unit run where 5 succeeded means the retry plans and
executes only 2 units.

**Interaction with persistent coordinator session (Phase 4):** When
`coordinator.persistent: true`, same-session continuity is handled natively —
the Opus session remembers the prior plan and can reference it directly without
memory search. `detect_continuity()` remains necessary for: (a) cold starts
(new editor session), (b) after session restart or `/compact`, (c) explicit
`run:{run_id}` references from the user pointing to runs from previous sessions.

### 10.4 Mechanical Tier Enrichment (Memory → Fewer Escalations)

For Reasoning-tier agents on Sonnet, memory enrichment is helpful but not
critical. For Mechanical-tier agents on a local 7B, memory enrichment is the
difference between success and escalation.

```
AgentRunner::enrich_prompt(unit, memory)
  │
  ├─ For Reasoning/Execution tier:
  │   memory.search_context[_filtered](namespaces, unit.description, 5[, filter])
  │   → general enrichment, appended to prompt
  │
  └─ For Mechanical tier:
     memory.search_context(namespaces, unit.description, 5)
     PLUS: explicit interface lookups:
       For each file in unit.scope.read_only:
         memory.search(namespace, "interface: {filename}", 2)
         → if found, include interface description in prompt
         → if not found, include raw file content

     The enriched prompt for the 7B includes:
       1. coordinator_instructions (terse, precise)
       2. Memory-derived interface descriptions (pattern to follow)
       3. File contents from scope (context for changes)
       4. Example from prior agent result if available (concrete pattern)
```

**Compound effect:** As more runs complete, memory accumulates richer
interface descriptions. The mechanical tier's success rate improves over time
without any changes to the local model itself.

### 10.5 Privacy-Aware Memory Protocol

**The problem:** When agents working on `LocalOnly` files store results in
memory, subsequent coordinator calls to Opus could retrieve that content,
violating privacy.

**The solution:** A dedicated `privacy` column in the schema (not JSON string
matching in the `metadata` field) and a `PrivacyFilter` on all API-bound queries.

**Schema migration** (added to `memory/schema.rs`):

```sql
-- Migration v2: add privacy column
ALTER TABLE memories ADD COLUMN privacy TEXT NOT NULL DEFAULT 'public';
CREATE INDEX IF NOT EXISTS idx_memories_privacy ON memories(privacy);
```

**Storage with privacy:**

```rust
// In SwarmPipeline::store_agent_result()
let privacy = match unit.privacy {
    PrivacyLevel::LocalOnly => "local_only",
    PrivacyLevel::Public => "public",
};

memory.store_with_privacy(
    namespace,
    &format!("agents:{}:{}", run_id, unit.id),
    &result_summary,
    privacy,
    Some(&metadata_json),
).await;
```

**Filtered query — new MemoryStore method:**

```rust
impl MemoryStore {
    /// Search with privacy filtering.
    /// Filtering happens in SQL (WHERE privacy != 'local_only') for
    /// ExcludeLocalOnly, avoiding post-query string matching.
    pub async fn search_context_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> String {
        // 1. Embed query OUTSIDE lock
        // 2. BRIEF LOCK: SELECT with optional WHERE privacy clause
        // 3. Cosine similarity + top-K OUTSIDE lock
        // 4. Format results
    }
}
```

**Usage across the pipeline:**

| Caller | Filter | Rationale |
|--------|--------|-----------|
| Coordinator (Opus) | `ExcludeLocalOnly` | API-bound |
| DiffReviewer (Sonnet) | `ExcludeLocalOnly` | API-bound |
| MergeResolver (Opus) | `ExcludeLocalOnly` | API-bound |
| AgentRunner for API models | `ExcludeLocalOnly` | API-bound |
| AgentRunner for local model | `IncludeAll` | Local — private content stays local |
| RepoContextBuilder | `ExcludeLocalOnly` | Feeds into coordinator prompt |
| TUI chat enrichment | `ExcludeLocalOnly` | Chat goes to API |
| `/remember` command | N/A (write path) | User-stored, default privacy: public |

**Verification summaries are always `Public`:** They contain only aggregate
statistics (pass/fail counts, escalation counts), never file content.

### 10.6 DiffReviewer Memory Enrichment

When Sonnet reviews diffs from lower tiers, coordinator_instructions are terse
by design (optimized for the 7B). Memory supplements the review with richer
context: prior agent summaries describing the broader refactor goal, interface
contracts from previous runs, and `/remember` entries about architectural
decisions.

**Token budget interaction:** Memory entries count against `maxDiffTokens`.
If adding memory would push over the limit, memory entries are truncated first
(diffs are essential; memory is supplementary).

### 10.7 Namespace Extensions

The existing namespace conventions from MEMORY.md remain. The tiered
architecture adds new key patterns within the existing project namespace:

| Key pattern | Writer | Content | Privacy |
|-------------|--------|---------|---------|
| `agents:{run_id}:{unit_id}` | SwarmPipeline | Task desc + modified files + summary | Inherits from WorkUnit |
| `verification:{run_id}` | SwarmPipeline | Aggregate verification results | Always Public |
| `tiers:{run_id}` | SwarmPipeline | Per-tier accuracy stats | Always Public |
| `user:{timestamp}` | `/remember` | User-provided text | Always Public |

> **Note for ARCHITECTURE.md update:** The `project:{name}:agents` namespace
> convention in ARCHITECTURE.md §11 is superseded by the `agents:{run_id}:{unit_id}`
> key pattern within the main project namespace. The `:agents` sub-namespace
> is redundant when keys encode agent provenance. ARCHITECTURE.md should be
> updated to reflect this.

### 10.8 Memory Retention and Pruning

The tier routing architecture writes significantly more memory entries than the
base system (3+ new entries per run vs occasional `/remember` and agent results).
Without pruning, brute-force cosine similarity degrades as entry count grows.

**Retention policy** (configured via `agent.memory`):

| Setting | Default | Effect |
|---------|---------|--------|
| `retentionMaxRuns` | 50 | Keep entries from the last N swarm runs per namespace |
| `retentionMaxAgeDays` | 30 | Delete entries older than N days |

**Implementation:**

```rust
impl MemoryStore {
    /// Delete stale swarm-generated entries. Called after each swarm run.
    /// Preserves all `user:{timestamp}` entries (explicit /remember).
    /// Preserves entries that don't match swarm key patterns.
    pub async fn prune(&self, namespace: &str, config: &RetentionConfig) -> Result<usize>;
}
```

**Pruning runs after Phase 4** (post-storage of new results). It is best-effort
— failures are logged but never fail the pipeline.

**Scaling note:** With `retentionMaxRuns: 50` and an average of 9 entries per
run, the maximum swarm-generated entries per namespace is ~450. Combined with
user entries, this stays well under the 50K brute-force ceiling described in
MEMORY.md. If usage patterns exceed this, an ANN index (e.g., sqlite-vss or
a custom HNSW) should be added as a separate effort.

### 10.9 Entry Versioning

Memory entries carry a `format_version` in metadata to support schema evolution:

```rust
#[derive(Serialize, Deserialize)]
pub struct EntryMetadata {
    pub privacy: PrivacyLevel,
    pub format_version: u8,     // Current: 1
    pub source: String,         // "swarm_pipeline", "remember_command", etc.
}
```

When reading entries, the pipeline checks `format_version`. Entries with
unknown versions are included in search results (they still have valid
embeddings) but excluded from structured parsing (e.g., `detect_continuity()`
won't parse a v2 entry with a v1 parser).

### 10.10 Compound Improvement Over Time

The memory system turns the tiered architecture from a static cost optimization
into an adaptive one:

| Phase | Memory state | Coordinator behaviour | Typical cost |
|-------|-------------|----------------------|-------------|
| Run 1 (cold) | Empty | Default tier calibration, full repo context | ~$0.10 |
| Runs 2-5 | Warming | References prior patterns, fewer escalations | ~$0.08 |
| Runs 5-10 | Calibrated | Well-calibrated tiers, 30-40% less context | ~$0.05 |
| Runs 10+ | Mature | Accurate plans, mechanical tier succeeds first-try | ~$0.04 |

This improvement requires no changes to the local model, no prompt engineering
iteration, and no manual configuration tuning.

### 10.11 Existing Integration Points (unchanged)

All existing memory integration points from MEMORY.md remain:
- `AgentRunner` enriches prompts with `search_context()` (now
  `search_context_filtered()` for API-bound agents)
- `SwarmPipeline` stores agent results (now with privacy + run_id + version)
- `/remember` command stores user-provided text
- Chat messages are NOT auto-stored

---

## 11. File Map (new and modified)

```
gaviero-core/src/
├── types.rs                    [MODIFIED] +ModelTier, +PrivacyLevel, +TierAnnotation
├── tree_sitter.rs              [MODIFIED] Extract find_enclosing_symbol() as pub helper
├── memory/
│   ├── store.rs                [MODIFIED] +search_context_filtered(), +store_with_privacy(),
│   │                                      +prune(), PrivacyFilter
│   └── schema.rs               [MODIFIED] +v2 migration: privacy column + index
├── swarm/
│   ├── models.rs               [MODIFIED] +WorkUnit tier fields, +format_version,
│   │                                      model field semantics doc, +AgentBackend::Ollama,
│   │                                      +AgentBackend::GeminiCli
│   ├── coordinator.rs          [NEW]      Coordinator, TaskDAG, ContinuityContext,
│   │                                      detect_continuity() (hybrid matching),
│   │                                      validate_dag(), RepoContextBuilder
│   ├── router.rs               [NEW]      TierRouter (model override + tier routing),
│   │                                      ResolvedBackend
│   ├── privacy.rs              [NEW]      PrivacyScanner, PrivacyFilter
│   ├── ollama.rs               [NEW]      OllamaBackend (HTTP client)
│   ├── verify/
│   │   ├── mod.rs              [NEW]      VerificationStrategy, CombinedReport, exports
│   │   ├── structural.rs       [NEW]      StructuralVerifier, StructuralReport
│   │   ├── diff_review.rs      [NEW]      DiffReviewer (merge-base diffs), DiffReviewReport
│   │   ├── test_runner.rs      [NEW]      TestRunner (inherit_env), TestReport
│   │   └── combined.rs         [NEW]      CombinedVerification orchestrator
│   ├── pipeline.rs             [MODIFIED] Phase 0 + Phase 4, per-tier Semaphores
│   │                                      with dep-tier completion boundary,
│   │                                      privacy-aware storage, pruning, cost tracking
│   ├── runner.rs               [MODIFIED] +run_ollama(), model-override dispatch,
│   │                                      tier-specific memory enrichment
│   ├── planner.rs              [MODIFIED] Delegates to Coordinator
│   ├── merge.rs                [MODIFIED] Uses Opus for conflict resolution
│   ├── bus.rs                  [UNCHANGED]
│   └── validation.rs           [MODIFIED] +privacy/model override validation in Phase 1
├── observer.rs                 [MODIFIED] +10 new SwarmObserver callbacks + on_cost_update
└── acp/
    ├── session.rs              [NEW]      SessionMode, AcpSessionFactory (one_shot + persistent),
    │                                      persistent session lifecycle, compact_with_memory_backup,
    │                                      slash command forwarding, EOF recovery
    └── client.rs               [MODIFIED] extract_file_blocks() made pub for reuse,
                                           AcpSession refactored to support both lifecycle modes
                                           (stdin kept open in Persistent mode, ResultEvent as
                                           end-of-turn delimiter)

gaviero-tui/src/
├── panels/
│   └── swarm_dashboard.rs      [MODIFIED] Tier badges, verification progress,
│                                          live cost display, escalation log,
│                                          memory depth indicator
└── event.rs                    [MODIFIED] New Event variants for coordination,
                                           verification, and cost lifecycle

gaviero-cli/src/
└── main.rs                     [MODIFIED] Coordinator config passthrough
```

---

## 12. Hard Constraints (preserved + extended)

All existing architectural invariants from ARCHITECTURE.md §17 remain in force.
Additional constraints for the tier routing and verification systems:

1. **Privacy is deterministic** — `PrivacyScanner` pattern matching overrides
   all LLM-suggested privacy levels. `LocalOnly` tasks never escalate to API.
   `model` overrides on `LocalOnly` units must not specify API models (Phase 1
   validation rejects this).
2. **Coordinator is the only Opus call** — per-prompt. No other pipeline stage
   calls Opus except MergeResolver for conflict resolution. If the Opus call
   fails, the swarm run fails immediately — no degraded fallback.
3. **Local backend is fire-and-forget** — no streaming, no tool use. Prompt in,
   file blocks out. Keeps the Ollama integration surface minimal.
4. **Tier 3 is always optional** — every code path handles `mechanical.enabled = false`
   by falling back to Haiku. No feature flags or conditional compilation needed.
5. **File block protocol is universal** — all backends (ACP, Ollama) produce
   `<file path="...">...</file>` blocks. The Write Gate pipeline is backend-agnostic.
6. **Reviewer never writes** — DiffReviewer sessions use `WriteMode::RejectAll`.
7. **Verification runs post-merge** — Phase 4 operates on the final merged
   codebase. Escalation uses fresh worktree + incremental merge (→ §7.2).
8. **More thorough wins** — user-configured verification settings cannot be
   weakened by coordinator strategy selection.
9. **Escalation is bounded** — max 3 total attempts per unit (1 original +
   1 retry + 1 escalation). Diff review never re-reviews an escalated unit.
   Test re-runs are scoped to originally failing tests only. (→ §7)
10. **Memory privacy is transitive** — `LocalOnly` content stored in the
    `privacy` column must never reach API-bound contexts. All memory queries
    feeding into API prompts use `PrivacyFilter::ExcludeLocalOnly` (→ §10.5).
11. **Dependency tiers are sequential** — per-model-tier Semaphores control
    concurrency within a dependency tier, not across them. No unit from
    dependency tier N+1 starts until ALL units in tier N have completed.
12. **Swarm agents are always one-shot** — every `AgentRunner` execution,
    including escalation re-executions, uses `SessionMode::OneShot`
    (`claude --print`). Persistent sessions are reserved for the TUI chat
    and the optional persistent coordinator (Phase 4). This ensures clean
    worktree isolation, deterministic cost per agent, and no context leakage
    between unrelated WorkUnits.

---

## 13. Recommended Local Model

For the optional Tier 3, given the RTX 4070 (8GB VRAM) running alongside
the e5-small-v2 embeddings model:

**Primary choice: Qwen 2.5 Coder 7B (Q4_K_M)**

- Best HumanEval at 7B (88.4%), 128K context, 92+ languages
- Code-specific training aligns with mechanical subtask profile
- ~30-50 t/s on RTX 4070, fits comfortably with embeddings model
- `ollama pull qwen2.5-coder:7b`

**Lightweight alternative: Qwen 3.5 4B**

- For higher parallelism or VRAM pressure scenarios
- 262K context, multimodal, 55.8 LiveCodeBench
- ~3.4GB, leaves significant headroom
- Trades some code quality for throughput — acceptable given tasks are
  pre-decomposed by Opus

The choice can be changed at runtime via `agent.tiers.mechanical.ollamaModel`
without any code changes.

---

## 14. Required Updates to Companion Documents

This section lists changes needed in ARCHITECTURE.md and MEMORY.md to stay
consistent with this plan. These are not implemented by this plan — they are
a checklist for a separate document update pass.

### ARCHITECTURE.md

1. **§3 (Core Abstractions → WorkUnit):** Add `tier`, `privacy`,
   `coordinator_instructions`, `estimated_tokens`, `max_retries`,
   `escalation_tier`, `format_version` fields. Document `model` field's
   dual-mode semantics (single-agent override vs tier-routing default).
2. **§3 (AgentBackend):** Add `GeminiCli` and `Ollama { model, base_url }`
   variants.
3. **§9 (ACP Integration):** Rewrite to document the dual-mode `AcpSession`:
   - `SessionMode::OneShot` — existing `claude --print` behaviour, now the
     explicit default for swarm agents, reviewer, and merge resolver.
   - `SessionMode::Persistent` — new `claude` (no --print) mode for TUI chat
     and optional persistent coordinator. Document stdin keep-alive, `ResultEvent`
     as end-of-turn delimiter, slash command forwarding, EOF recovery.
   - `AcpSessionFactory` — new factory managing both modes, with persistent
     session pool keyed by purpose string.
   - `compact_with_memory_backup()` — interaction between session compaction
     and the memory system.
4. **§11 (Memory → Namespace Conventions):** Replace `project:{name}:agents`
   with the key patterns from §10.7 of this plan. The `:agents` sub-namespace
   is superseded.
5. **§11 (Memory → Integration Points):** Remove "Memory is
   `Option<Arc<MemoryStore>>` — `None` before M3" — the milestone qualifier
   is stale.
6. **§11 (Memory → File Paths / Model Cache):** Normalize path casing to
   lowercase `~/.cache/gaviero/models/` throughout (Linux is case-sensitive;
   `dirs::cache_dir()` produces lowercase).
7. **§2 (Module Map):** Add new modules: `acp/session.rs`,
   `swarm/coordinator.rs`, `swarm/router.rs`, `swarm/privacy.rs`,
   `swarm/ollama.rs`, `swarm/verify/*`.
8. **§15 (Concurrency Model → Spawned Background Tasks):** Add persistent
   session NDJSON reader (per-persistent-session lifetime, replaces the
   per-conversation reader for persistent sessions).

### MEMORY.md

1. **§SQLite Schema:** Add `privacy` column and index (migration v2).
2. **§MemoryStore API:** Add `search_context_filtered()`,
   `store_with_privacy()`, `prune()`.
3. **§File Paths:** Verify all paths use lowercase `gaviero` consistently.
