# Gaviero — Architecture

Terminal editor + headless CLI for AI agent orchestration. Rust 2024 edition.

**Binaries:** `gaviero` (TUI), `gaviero-cli` (headless)  
**Platform:** Linux primary, POSIX terminals  
**Build:** `cargo build` (workspace root)

---

## 1. Crate Topology

```
┌─────────────────────────────────────────────────────────┐
│                    Workspace Root                       │
├──────────────────────┬──────────────────────────────────┤
│                      │                                  │
▼                      ▼                      ▼            ▼
┌────────────┐  ┌────────────┐  ┌────────────┐  ┌─────────────┐
│gaviero-core│  │gaviero-tui │  │gaviero-cli │  │gaviero-dsl  │
│ (library)  │  │ (binary)   │  │ (binary)   │  │ (library)   │
└──────┬─────┘  └─────┬──────┘  └─────┬──────┘  └────────┬────┘
       │              │               │                  │
       │              └─────────────┬──┴──────────────────┘
       │                            │ (dsl depends on core)
       │              ┌─────────────┘
       │              │
       └──────┬───────┘
              │
       ┌──────▼───────────────────┐
       │ tree-sitter-gaviero      │
       │ (grammar + parser)       │
       └──────────────────────────┘
```

**Dependency rules:**
- `gaviero-core`: no UI dependencies, no DSL dependencies
- `gaviero-tui`, `gaviero-cli`: depend on `gaviero-core` + `gaviero-dsl`
- `gaviero-dsl`: depends on `gaviero-core` only
- `tree-sitter-gaviero`: pure grammar, re-exported from `gaviero-core::lib`

| Crate | Type | Responsibility | Key Dependencies |
|---|---|---|---|
| `gaviero-core` | Library (18 modules, ~14 KLOC) | Runtime: swarm, chat, memory, git, validation, write-gate | tokio, tree-sitter 0.25, git2, rusqlite+sqlite-vec, ort, petgraph, ropey, portable-pty |
| `gaviero-tui` | Binary (~30 KLOC) | Terminal UI: editor, panels, event routing, rendering | ratatui 0.30, crossterm 0.29, notify, core, dsl |
| `gaviero-cli` | Binary (~500 LOC) | Argument parsing, observer wiring, delegation to core | clap, tokio, core, dsl |
| `gaviero-dsl` | Library (~2 KLOC, 5 modules) | DSL compiler: lexer → parser → AST → CompiledPlan | logos, chumsky, miette, core |
| `tree-sitter-gaviero` | Grammar (~300 LOC JS + C) | Syntax tree and highlighting for `.gaviero` files | tree-sitter (build-time) |

---

## 2. Core Module Map

### `gaviero-core/src/` — 18 public modules

```
lib.rs                  Re-exports tree-sitter LANGUAGE_REGISTRY + 18 modules

types.rs                FileScope, WriteProposal, ModelTier, PrivacyLevel, DiffHunk,
                        StructuralHunk, NodeInfo, SymbolKind, TierAnnotation

workspace.rs            Workspace, WorkspaceFolder, settings cascade, namespace resolution

session_state.rs        SessionState, TabState, PanelState, StoredConversation

tree_sitter.rs          LANGUAGE_REGISTRY (16 langs), enrich_hunks(), language detection

diff_engine.rs          compute_hunks() — via `similar` crate -> Vec<DiffHunk>

write_gate.rs           WriteGatePipeline, WriteMode, ProposalStatus, proposal lifecycle

observer.rs             WriteGateObserver, AcpObserver, SwarmObserver trait definitions

scope_enforcer.rs       FileScope permission checks, ownership validation

git.rs                  GitRepo, WorktreeManager, GitCoordinator, FileStatus, git2 wrapper

query_loader.rs         Tree-sitter .scm query file discovery

acp/                    Claude subprocess protocol (NDJSON), session lifecycle, multi-turn
  ├─ session.rs         AcpSession, AgentOptions, spawn, polling
  ├─ protocol.rs        NDJSON event types, parsing, stream reconstruction
  ├─ client.rs          AcpPipeline, prompt enrichment, file block routing
  └─ factory.rs         AcpSessionFactory, session lifecycle manager

memory/                 Scoped semantic memory, embeddings, consolidation
  ├─ store.rs           MemoryStore (SQLite + sqlite-vec wrapper), CRUD
  ├─ scope.rs           MemoryScope (5-level hierarchy), WriteScope, Trust, MemoryType
  ├─ scoring.rs         SearchConfig, ScoredMemory, scoring formula (50% sim + 20% importance…)
  ├─ consolidation.rs   Consolidator (3-phase: triage → decay → promotion)
  ├─ embedder.rs        Embedder trait, vector dimensions, model_id()
  ├─ onnx_embedder.rs   OnnxEmbedder (ort + tokenizers), nomic-embed-text-v1.5
  └─ model_manager.rs   Model download + cache management

repo_map/               File ranking + context graph
  ├─ mod.rs             RepoMap, build(), rank_for_agent()
  ├─ store.rs           GraphStore, FileNode, DirectedEdge
  └─ pagerank.rs        Personalized PageRank implementation

swarm/                  Multi-agent orchestration engine
  ├─ models.rs          WorkUnit, AgentManifest, SwarmResult, MergeResult
  ├─ plan.rs            CompiledPlan, PlanNode, petgraph DAG
  ├─ pipeline.rs        Execute, 6-phase orchestration (validate → execute → merge → verify → cleanup → consolidate)
  ├─ coordinator.rs     Natural-language → DAG planner (Opus-powered)
  ├─ validation.rs      Scope overlap checks, Kahn's topological sort
  ├─ router.rs          TierRouter: (ModelTier, PrivacyLevel) → ResolvedBackend
  ├─ privacy.rs         PrivacyScanner: glob-based privacy override
  ├─ execution_state.rs ExecutionState, NodeStatus, checkpoint/resume
  ├─ merge.rs           Git merge + conflict resolution (Claude-powered)
  ├─ bus.rs             AgentBus: broadcast + targeted inter-agent messaging
  ├─ board.rs           SharedBoard: agent discovery board (tagged findings)
  ├─ context.rs         RepositoryContext: file collection + ref extraction
  ├─ backend/           AgentBackend trait + implementations
  │   ├─ mod.rs         AgentBackend trait, UnifiedStreamEvent, CompletionRequest
  │   ├─ shared.rs      Model-spec parsing, prompt enrichment, provider detection
  │   ├─ executor.rs    Stream event processing (text collection or write-gate routing)
  │   ├─ claude_code.rs ClaudeCodeBackend (ACP subprocess)
  │   ├─ ollama.rs      OllamaStreamBackend (HTTP SSE)
  │   ├─ codex.rs       CodexBackend (local OpenAI-like)
  │   ├─ mock.rs        MockBackend (testing)
  │   └─ runner.rs      run_backend() orchestrator
  └─ verify/            Verification strategies
      ├─ structural.rs  Tree-sitter parse checks (ERROR/MISSING nodes)
      ├─ diff_review.rs LLM diff review (batched per-unit/tier)
      ├─ test_runner.rs Test command execution (cargo test, pytest, jest)
      └─ combined.rs    Verification orchestrator (sequential, early-exit, escalation)

iteration/              Retry + escalation engine
  ├─ mod.rs             IterationEngine, IterationConfig, Strategy enum
  ├─ engine.rs          Retry loops, best-of-N sampling, test-first mode
  └─ converter.rs       TestGenerator: failing test generation (TDD)

validation_gate/        Syntax + semantic validation gates
  ├─ mod.rs             ValidationPipeline, ValidationGate trait
  ├─ tree_sitter_gate.rs TreeSitterGate: parse-error detection
  └─ cargo_check_gate.rs CargoCheckGate: Rust compile checks

indent/                 Tree-sitter + heuristic indentation
  ├─ mod.rs             compute_indent() dispatcher
  ├─ tree_sitter.rs     Tree-sitter query-based indent
  ├─ hybrid.rs          Heuristic fallback (indent patterns)
  └─ bracket.rs         Bracket-counting final fallback

terminal/               Embedded PTY + shell session management
  ├─ mod.rs             TerminalManager, TerminalInstance
  ├─ pty.rs             PTY spawn + I/O (portable-pty)
  └─ osc133.rs          OSC 133 prompt/command parsing

repla...               (Future: REPL subsystem, if applicable)
```

---

## 3. Core Abstractions

### `FileScope` (types.rs:124)

Agent permission boundary. All agent writes checked against this.

```rust
pub struct FileScope {
    pub owned_paths: Vec<String>,              // Writable files/dirs
    pub read_only_paths: Vec<String>,          // Read-only files/dirs
    pub interface_contracts: HashMap<String, String>, // API contracts to preserve
}
```

**Semantics:**
- Owned paths are writable; all descendant files inherit permission
- Read-only paths cannot be modified
- Overlap detection: no two agents can own the same file
- `interface_contracts`: specify APIs (e.g., `fn parse()`) that must not change signature

**Used by:**
- `write_gate`: scope enforcement
- `swarm::validation`: overlap detection
- `swarm::privacy`: privacy classification
- `memory/scope`: module-level namespace routing

### `WorkUnit` (swarm/models.rs:52)

Single execution task for one swarm agent. Fully specifies what an agent should do.

```rust
pub struct WorkUnit {
    pub id: String,
    pub description: String,
    pub scope: FileScope,
    pub depends_on: Vec<String>,               // Prerequisite unit IDs
    pub coordinator_instructions: String,      // Task instructions from planner
    pub model: Option<String>,                 // Per-unit model override
    pub tier: ModelTier,                       // Cheap | Expensive
    pub privacy: PrivacyLevel,                 // Public | LocalOnly
    pub max_retries: u8,                       // Retries before escalation
    pub escalation_tier: Option<ModelTier>,    // Tier on failure
    
    // Memory routing
    pub read_namespaces: Option<Vec<String>>,  // Namespaces to read from
    pub write_namespace: Option<String>,       // Namespace to write to
    pub memory_importance: Option<f32>,        // Importance 0.0-1.0 for consolidation
    pub staleness_sources: Vec<String>,        // Paths triggering staleness decay
    pub memory_read_query: Option<String>,     // Custom memory search query
    pub memory_read_limit: Option<usize>,      // Custom result limit
    
    // Context expansion
    pub impact_scope: bool,                    // Auto-expand read_only via blast-radius
    pub context_callers_of: Vec<String>,       // Files for caller graph
    pub context_tests_for: Vec<String>,        // Paths for test queries
    pub context_depth: u32,                    // BFS depth (default: 2)
}
```

**Created by:**
- `gaviero-dsl::compiler`: from DSL AgentDecl
- `swarm::coordinator`: from natural-language task decomposition
- `gaviero-cli`: synthetic WorkUnit from `--task` flag

### `CompiledPlan` (swarm/plan.rs:18)

Immutable DAG representing a complete swarm execution plan.

```rust
pub struct CompiledPlan {
    pub graph: DiGraph<PlanNode, DependencyEdge>,  // petgraph DAG
    pub max_parallel: Option<usize>,               // Concurrency cap
    pub source_file: Option<PathBuf>,              // Source .gaviero file
    pub iteration_config: IterationConfig,         // Strategy + retry + escalation
    pub verification_config: VerificationConfig,   // Compile/test/review flags
    pub loop_configs: Vec<LoopConfig>,             // Loop termination conditions
}

pub struct PlanNode {
    pub work_unit: WorkUnit,
    pub status: NodeStatus,
}
```

**Key methods:**
- `work_units_ordered() -> Vec<WorkUnit>`: Kahn's topo-sort of nodes
- `from_work_units(units: Vec<WorkUnit>) -> Result<Self>`: flat list → DAG
- `hash() -> String`: stable plan hash (checkpoint name)

**Used by:**
- `swarm::pipeline::execute()`: input
- `gaviero-dsl::compile()`: output
- `swarm::coordinator`: planning output

### `UnifiedStreamEvent` (swarm/backend/mod.rs:84)

Normalized event from any backend (Claude Code, Ollama, Codex, Mock).

```rust
pub enum UnifiedStreamEvent {
    TextDelta { content: String },
    ThinkingDelta { content: String },
    ToolCallStart { tool_name: String, tool_use_id: String },
    ToolCallDelta { tool_use_id: String, input_delta: String },
    ToolCallEnd { tool_use_id: String },
    FileBlock { path: PathBuf, content: String },
    Usage { input_tokens: u32, output_tokens: u32, cost_usd: f64 },
    Error { message: String, is_auth_error: bool },
    Done { stop_reason: StopReason },
}
```

**Produced by:**
- `ClaudeCodeBackend`: parses ACP protocol
- `OllamaStreamBackend`: parses Ollama JSON
- `CodexBackend`: parses OpenAI stream format
- `MockBackend`: synthetic events (testing)

**Consumed by:**
- `backend::executor::complete_to_text()`: collects text
- `backend::executor::complete_to_write_gate()`: routes file blocks to write gate
- `AcpPipeline::propose_write()`: inserts proposals

### `WriteProposal` (types.rs:85)

File modification proposal with structural context and acceptance tracking.

```rust
pub struct WriteProposal {
    pub id: u64,
    pub source: String,                    // Agent ID or "user"
    pub file_path: PathBuf,
    pub original_content: String,
    pub proposed_content: String,
    pub structural_hunks: Vec<StructuralHunk>,  // DiffHunk + AST context
    pub status: ProposalStatus,            // Pending | PartiallyAccepted | Accepted | Rejected
}

pub struct StructuralHunk {
    pub hunk: DiffHunk,
    pub enclosing_node: Option<String>,    // Enclosing function/struct/class
    pub node_kind: Option<String>,         // "function_definition", "struct_item", etc.
}
```

**Lifecycle:**
1. `write_gate.insert_proposal(proposal)`: enqueued in Interactive mode
2. `on_proposal_created()`: observer notified
3. User reviews via TUI (accept/reject per hunk or by node)
4. `write_gate.finalize_proposal(id)`: assemble final content from accepted hunks
5. `on_proposal_finalized()`: disk write or deferred batch

### `AgentBackend` trait (swarm/backend/mod.rs:45)

Abstraction over provider implementations (Claude, Ollama, Codex).

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync + 'static {
    async fn stream_completion(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;

    fn capabilities(&self) -> Capabilities;
    fn name(&self) -> &str;
    async fn health_check(&self) -> Result<()>;
}

pub struct CompletionRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

pub struct Capabilities {
    pub supports_file_blocks: bool,
    pub supports_thinking: bool,
    pub supports_vision: bool,
}
```

**Implementations:**
- `ClaudeCodeBackend`: spawns `claude` CLI subprocess
- `OllamaStreamBackend`: HTTP SSE to Ollama `/api/generate`
- `CodexBackend`: OpenAI-compatible local endpoint
- `MockBackend`: testing fixture

### `MemoryScope` (memory/scope.rs:24)

Resolved scope chain for hierarchical memory search.

```rust
pub struct MemoryScope {
    pub global_db: PathBuf,                    // ~/.config/gaviero/memory.db
    pub workspace_db: Option<PathBuf>,         // <workspace>/.gaviero/memory.db
    pub workspace_id: Option<String>,          // Workspace name/hash
    pub repo_id: Option<String>,               // Git repo name
    pub module_path: Option<String>,           // Relative path in repo
    pub run_id: Option<String>,                // Ephemeral run ID
}

impl MemoryScope {
    pub fn levels(&self) -> Vec<ScopeLevel> {
        // [run, module, repo, workspace, global]
    }
}
```

**Used by:**
- `memory::store::search_scoped()`: cascade from narrowest to widest
- `memory::store::store_scoped()`: determine write target
- `consolidation`: promotion thresholds

### Observer Traits (observer.rs:1)

Callback interfaces for runtime events.

```rust
pub trait WriteGateObserver: Send + Sync {
    fn on_proposal_created(&self, proposal: &WriteProposal);
    fn on_proposal_updated(&self, id: u64, status: ProposalStatus);
    fn on_proposal_finalized(&self, id: u64, status: ProposalStatus);
}

pub trait AcpObserver: Send + Sync {
    fn on_stream_chunk(&self, chunk: &str);
    fn on_tool_call_started(&self, tool_name: &str, tool_use_id: &str);
    fn on_streaming_status(&self, message: &str);
    fn on_message_complete(&self, stats: &MessageStats);
}

pub trait SwarmObserver: Send + Sync {
    fn on_phase_changed(&self, phase: &str);
    fn on_agent_state_changed(&self, unit_id: &str, status: AgentStatus);
    fn on_tier_started(&self, tier: usize, units: &[WorkUnit]);
    fn on_completed(&self, result: &SwarmResult);
    // ... 8+ more callbacks
}
```

**Implemented by:**
- `gaviero-tui`: bridges to event channel → TUI event loop
- `gaviero-cli`: formats events to stderr
- Testing: mock implementations

---

## 4. Data Flow: Agent Write Proposal

Every agent file modification flows through this pipeline to enforce scope safety.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Agent (AcpSession/OllamaStream)              │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         │ <file path="src/foo.rs">...content...</file>
                         │ (detected in streaming output)
                         │
        ┌────────────────▼────────────────┐
        │ acp/client.rs                   │
        │ AcpPipeline::propose_write()    │
        └────────────────┬────────────────┘
                         │
        ┌────────────────▼──────────────────────────────┐
        │ 1. BRIEF LOCK: write_gate                     │
        │    is_scope_allowed(agent_id, path)? → NO    │
        │    Release lock immediately                   │
        └────────────────┬──────────────────────────────┘
                         │
        ┌────────────────▼──────────────────────────────┐
        │ 2. NO LOCK: diff_engine                      │
        │    original = fs::read_to_string(path)       │
        │    hunks = compute_hunks(original, content)  │
        └────────────────┬──────────────────────────────┘
                         │
        ┌────────────────▼──────────────────────────────┐
        │ 3. NO LOCK: tree_sitter                      │
        │    enrich_hunks(hunks, original, language)   │
        │    -> StructuralHunk[] with AST context      │
        └────────────────┬──────────────────────────────┘
                         │
        ┌────────────────▼──────────────────────────────┐
        │ 4. BRIEF LOCK: write_gate                    │
        │    insert_proposal(proposal)                 │
        │    ├─ Interactive → queue, fire observer     │
        │    │   → TUI shows diff review               │
        │    ├─ AutoAccept → accept all hunks          │
        │    ├─ Deferred → accumulate for batch        │
        │    └─ RejectAll → discard silently           │
        │    Release lock                              │
        └────────────────┬──────────────────────────────┘
                         │
        ┌────────────────▼──────────────────────────────┐
        │ 5. NO LOCK: disk I/O                         │
        │    if AutoAccept: fs::write(path, content)   │
        └────────────────┴──────────────────────────────┘

Lock discipline: WriteGatePipeline Mutex held for O(1) operations only.
All tree-sitter parsing, diff computation, and I/O happen outside lock.
```

---

## 5. Data Flow: Swarm Execution

Complete 6-phase orchestration from CompiledPlan to SwarmResult.

```
┌──────────────────────────────────────────────────────────────────┐
│ swarm::pipeline::execute(plan, config, memory, observer, ...)    │
└──────────────────┬───────────────────────────────────────────────┘
                   │
   ┌───────────────▼────────────────┐
   │ PHASE 1: VALIDATE              │
   ├────────────────────────────────┤
   │ validate_scopes()              │  O(n²) pairwise check: no
   │   → FileScope overlap check    │  owned_path conflicts
   │                                │
   │ work_units_ordered()           │  Kahn's topological sort
   │   → linear dependency order    │
   │                                │
   │ dependency_tiers()             │  Vec<Vec<WorkUnit>>
   │   → parallel execution groups  │  group by dependencies
   └───────────────┬────────────────┘
                   │
   ┌───────────────▼──────────────────────────────────────────┐
   │ PHASE 2: EXECUTE (for each tier, sequential)             │
   ├────────────────────────────────────────────────────────┐
   │ Tier N: [UnitA, UnitB, UnitC] (bounded by Semaphore)  │
   │                                                        │
   │   For each WorkUnit (concurrent, bounded):            │
   │   ├─ git_repo.checkout(gaviero/{unit_id})             │
   │   │                                                    │
   │   └─ IterationEngine::run(unit, config)               │
   │       ├─ [if test_first] generate failing tests       │
   │       │                                                │
   │       └─ FOR attempt in 0..max_attempts:              │
   │           ├─ if attempt >= escalate_after             │
   │           │   tier = escalation_tier                  │
   │           │                                            │
   │           └─ run_backend(unit, tier, attempt)         │
   │               ├─ build_prompt()                       │
   │               │  ├─ scoped memory search              │
   │               │  ├─ file_scope clause                 │
   │               │  ├─ repo_map context (PageRank)       │
   │               │  ├─ shared_board discoveries          │
   │               │  └─ corrective feedback (on retry)    │
   │               │                                        │
   │               ├─ backend.stream_completion(request)   │
   │               │   → Stream<UnifiedStreamEvent>        │
   │               │                                        │
   │               ├─ for each FileBlock in stream:        │
   │               │   write_gate.insert_proposal()        │
   │               │                                        │
   │               └─ ValidationPipeline::run(modified)    │
   │                   ├─ TreeSitterGate (parse check)     │
   │                   ├─ CargoCheckGate (compile check)   │
   │                   └─ TestRunnerGate (if enabled)      │
   │                       PASS → next agent               │
   │                       FAIL → corrective → retry       │
   │   Checkpoint ExecutionState after unit completion     │
   │                                                        │
   └───────────────┬──────────────────────────────────────┘
                   │
   ┌───────────────▼────────────────┐
   │ PHASE 3: MERGE (if use_worktrees) │
   ├────────────────────────────────┤
   │ For each successful branch:    │
   │   git merge --no-ff main       │
   │   on conflict → MergeResolver  │
   │   (Claude queries for advice)  │
   └───────────────┬────────────────┘
                   │
   ┌───────────────▼────────────────┐
   │ PHASE 4: VERIFY (optional)     │
   ├────────────────────────────────┤
   │ per VerificationStrategy:      │
   │  ├─ StructuralOnly             │
   │  ├─ DiffReview (LLM batched)   │
   │  ├─ TestSuite (run_tests)      │
   │  └─ Combined (sequential)      │
   │ Escalation on failure          │
   └───────────────┬────────────────┘
                   │
   ┌───────────────▼────────────────┐
   │ PHASE 5: CLEANUP               │
   ├────────────────────────────────┤
   │ WorktreeManager::teardown_all()│
   │ Delete gaviero/* branches      │
   └───────────────┬────────────────┘
                   │
   ┌───────────────▼────────────────┐
   │ PHASE 6: MEMORY CONSOLIDATION  │
   ├────────────────────────────────┤
   │ Consolidator::consolidate_run()│
   │  ├─ triage (importance >= 0.4) │
   │  │  → promote to module/repo   │
   │  ├─ decay (30-day half-life)   │
   │  └─ cross-scope promotion      │
   │     (3+ module hit → repo)     │
   └───────────────┬────────────────┘
                   │
                   ▼
   ┌───────────────────────────────┐
   │ Return SwarmResult            │
   │  ├─ manifests[]               │
   │  ├─ merge_results[]           │
   │  ├─ success: bool             │
   │  └─ pre_swarm_sha: String     │
   └───────────────────────────────┘
```

---

## 6. Concurrency Model

Single shared `tokio` runtime. Coordinated lock discipline per subsystem.

### Sync Primitives

```rust
Arc<tokio::sync::Mutex<T>>         // WriteGatePipeline, MemoryStore (SQLite)
Arc<dyn Observer>                  // Observer trait objects across tasks
mpsc::UnboundedChannel<Event>      // TUI event routing (single consumer)
broadcast::channel                 // AgentBus (multi-consumer agent messaging)
Semaphore                          // Parallel agent concurrency bound
```

### Lock Discipline Rules

| Subsystem | Lock | Rule | Rationale |
|---|---|---|---|
| WriteGatePipeline | Mutex<HashMap> | Never hold across I/O or parsing | Prevents stalls during slow ops |
| MemoryStore | Mutex<rusqlite::Connection> | Embed outside lock, brief DB ops | ONNX inference is CPU-heavy |
| ExecutionState | Mutex<Vec<NodeStatus>> | Checkpoint after each node | Resumable on crash |
| AgentBus | broadcast::channel | Lock-free, async-friendly | No blocking on message send |

**Per-subsystem:** see sections 7 (Memory), 8 (Write Gate), 9 (Backend).

### Shared State Across Tasks

1. **WriteGatePipeline Mutex:** write gate proposals
2. **MemoryStore Mutex:** SQLite connection
3. **Observer Arc:** trait object clones sent to background tasks
4. **Event channel:** TUI main loop single receiver
5. **AgentBus broadcast:** agent-to-agent messaging

**Golden rule:** No Mutex held across await points, tree-sitter parse, or fs I/O.

---

## 7. Memory System

Hierarchical, scoped embeddings + consolidation + code graph.

### Scope Hierarchy (Broadest → Narrowest)

```
Level 0: Global        ~/.config/gaviero/memory.db (cross-workspace)
  │
  └─ Level 1: Workspace  <workspace>/.gaviero/memory.db (business-level)
      │
      └─ Level 2: Repo    Single git repo (repo_id)
          │
          └─ Level 3: Module  Crate/subdir (FileScope.owned_paths)
              │
              └─ Level 4: Run    Ephemeral per-execution (run_id)
```

**Search:** cascades narrowest → widest, early-exits at confidence threshold (0.70).

### Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                      MemoryStore                             │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐    │
│  │ rusqlite     │  │  Embedder    │  │ Consolidator   │    │
│  │ Connection   │  │  (ONNX)      │  │  (3-phase)     │    │
│  │ (Mutex)      │  │              │  │                │    │
│  └──────────────┘  └──────────────┘  └────────────────┘    │
│                                                              │
│  Tables:                                                     │
│  ├─ memories (scope_level, scope_path, repo_id,            │
│  │            module_path, run_id, content_hash,            │
│  │            memory_type, trust, tag)                      │
│  ├─ vec_memories_scoped (embedding partitioned by level)   │
│  ├─ memories_fts (FTS5 full-text search)                   │
│  ├─ memory_access_log (cross-scope promotion heuristics)   │
│  ├─ episodes (agent run tracking)                          │
│  └─ graph_state (code knowledge graph)                     │
└──────────────────────────────────────────────────────────────┘
```

### Search: Data Flow

```
memory.search_scoped(SearchConfig { query, max_results, … })
│
├─ 1. NO LOCK: embedder.embed(query) → Vec<f32>  [CPU: ONNX]
│
├─ 2. CASCADE (narrowest → widest):
│    FOR level in [run, module, repo, workspace, global]:
│      │
│      ├─ BRIEF LOCK: vec_search_at_level()
│      │   SELECT from vec_memories_scoped WHERE scope_level = ?
│      │   Release lock
│      │
│      ├─ [if use_fts] BRIEF LOCK: fts_search_at_level()
│      │   SELECT from memories_fts INNER JOIN memories
│      │   Release lock
│      │
│      ├─ NO LOCK: merge_rrf(vec_results, fts_results)
│      │   RRF: 70% vector + 30% FTS
│      │
│      ├─ NO LOCK: score each candidate
│      │   final_score = (sim*0.50 + importance*0.20 + recency*0.15 + 0.15)
│      │                 * scope_weight * trust_weight
│      │
│      └─ EARLY EXIT: if best_score > 0.70 → stop widening
│
├─ 3. NO LOCK: deduplicate by content_hash across levels
│
└─ 4. Return top-K ScoredMemory (sorted by score)
```

**Lock discipline:** Embeddings happen before lock. DB lock held briefly for each level's query only.

### Write: Data Flow

```
memory.store_scoped(scope: WriteScope, content, meta: WriteMeta)
│
├─ 1. NO LOCK: embedder.embed(content) → Vec<f32>
│
├─ 2. NO LOCK: content_hash = SHA-256(normalized(content))
│
├─ 3. BRIEF LOCK: deduplication check
│    SELECT FROM memories WHERE content_hash = ?
│    AND scope_level <= current_level
│    ├─ Exact match same scope → reinforce (update importance, access_count)
│    │   → StoreResult::Deduplicated(id)
│    ├─ Exact match broader scope → skip
│    │   → StoreResult::AlreadyCovered
│    └─ No match → INSERT with scope metadata
│       → StoreResult::Inserted(id)
│
└─ 4. BRIEF LOCK: insert into vec_memories_scoped + memories_fts
     (FTS trigger auto-updates search index)
```

### Consolidation: 3-Phase Pipeline

Runs after each swarm execution (Phase 6).

```
Consolidator::consolidate_run(run_id, repo_id)
│
├─ Phase 1: RUN TRIAGE
│   ├─ query_by_run(run_id) → Vec<ScoredMemory>
│   ├─ FOR each memory:
│   │   if importance >= 0.4:
│   │     store_scoped(Module or Repo, content, WriteMeta::consolidation())
│   │     → Inserted | Deduplicated | AlreadyCovered
│   └─ delete_by_run(run_id) ← remove ephemeral run-level entries
│
├─ Phase 2: IMPORTANCE DECAY + PRUNING
│   ├─ decay_and_prune()
│   ├─ Exponential: importance *= exp(-0.023 * days_since_access)
│   │   Half-life: 30 days
│   └─ Delete entries below threshold
│
└─ Phase 3: CROSS-SCOPE PROMOTION
    ├─ find_promotion_candidates(min_cross_hits=3)
    │   Module memories accessed by 3+ different modules
    ├─ promote to Repo scope
    └─ apply 1.2x importance boost
```

### Embedder

- **Interface:** `embed(text) → Vec<f32>`, `dimensions()`, `model_id()`
- **Implementation:** OnnxEmbedder
  - **Model:** nomic-embed-text-v1.5 (768 dimensions)
  - **Runtime:** `ort` (ONNX) + `tokenizers` crate
  - **Pooling:** mean pooling + L2 norm
- **Vector search:** sqlite-vec `vec0` virtual table, cosine distance

---

## 8. Write Gate

Enforces file-write safety through scope validation and interactive review.

### WriteMode Enum

| Mode | Behavior | Used by |
|---|---|---|
| `Interactive` | Queue proposals → TUI review (accept/reject per hunk) | Editor (chat mode) |
| `AutoAccept` | Validate scope, immediately accept and write | Swarm (CI-friendly) |
| `Deferred` | Accumulate for batch review after agent turn | TUI agent chat |
| `RejectAll` | Silently discard all proposals | Safety fallback |

### Proposal Lifecycle

```
WorkUnit asks to write(file_path, content)
│
├─ 1. BRIEF LOCK: is_scope_allowed(unit_id, file_path)?
│    Release lock immediately
│    ├─ Scope check PASS → continue
│    └─ Scope check FAIL → reject, observer callback
│
├─ 2. NO LOCK: compute diff
│    original = fs::read_to_string(file_path)
│    hunks = diff_engine::compute_hunks(original, content)
│    structural = tree_sitter::enrich_hunks(hunks, original, lang)
│
├─ 3. BRIEF LOCK: insert_proposal()
│    ├─ Interactive → queue + on_proposal_created() → TUI shows diff review
│    ├─ AutoAccept → accept all + return Some((path, content))
│    ├─ Deferred → accumulate + return None
│    └─ RejectAll → discard + return None
│    Release lock
│
└─ 4. [if Auto/Rejected] NO LOCK: disk write or skip
```

### Structural Awareness

Each hunk encodes its AST context:

```rust
pub struct StructuralHunk {
    pub hunk: DiffHunk,
    pub enclosing_node: Option<String>,  // e.g., "parse_config"
    pub node_kind: Option<String>,       // e.g., "function_definition"
}
```

**UI capability:** `accept_node(proposal_id, "parse_config")` accepts all hunks within that named symbol.

---

## 9. Error Handling

### Error Taxonomy

| Type | Crate | Recoverable | Handling |
|---|---|---|---|
| `anyhow::Error` | All | Context-dependent | Log + propagate |
| `DslError` | gaviero-dsl | Validation-time | Miette diagnostics with source spans |
| `ScopeError` | gaviero-core | Runtime | Reject agent write, log, continue |
| `ValidationFailure` | validation_gate | Recovery possible | Corrective prompt → retry |
| `MergeConflict` | swarm::merge | Interactive | Claude resolution or user choice |
| `WorktreeSetupError` | git | Retry possible | Cleanup → retry tier |

### Strategies

1. **Parse/Compile errors:** Return `miette::Report` with spans
2. **Agent execution failure:** `AgentStatus::Failed(reason)`, escalate if configured
3. **Scope violations:** Proposal rejected, logged, no retry
4. **Validation gate failure:** Corrective feedback → same agent retries
5. **Memory init failure:** Non-fatal — `Option<Arc<MemoryStore>>`, continue without memory
6. **Consolidation failure:** Best-effort, log, continue
7. **Worktree cleanup failure:** Logged, does not block completion

---

## 10. Hard Architectural Constraints

Invariants. Do not violate without extensive discussion.

1. **Write Gate Mandatory**
   - All agent writes through `WriteGatePipeline`
   - No direct `fs::write()` from agent execution
   - Scope checked before any disk I/O

2. **git2 Only**
   - Never shell out to `git`
   - All git operations via `git2` crate
   - Worktree management in `git.rs`

3. **Tree-Sitter for Structure**
   - Syntax analysis: tree-sitter queries
   - Highlighting: tree-sitter queries (not regex)
   - Indentation: tree-sitter + heuristic + bracket fallback
   - 16 language grammar registry

4. **Mutex-Wrapped SQLite**
   - `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`
   - All DB methods async/awaitable
   - Embedding outside lock before store/search

5. **Core/UI Separation**
   - All runtime logic in `gaviero-core`
   - TUI owns rendering + input dispatch
   - CLI owns arg parsing + observer wiring
   - Core testable without UI

6. **Single Event Channel (TUI)**
   - One `mpsc::UnboundedChannel<Event>` to main loop
   - No direct state mutation from background tasks
   - All external events become `Event` enum variants

7. **AutoAccept in Swarm**
   - `WriteMode::AutoAccept` during swarm execution
   - User reviews merged results post-swarm
   - Enables fully autonomous CI runs

8. **Observer-Only Coupling**
   - Core never imports TUI/CLI types
   - All communication via trait objects
   - Backends + providers opaque to orchestration

9. **Provider-Agnostic Backend**
   - `AgentBackend` + `UnifiedStreamEvent` decouple orchestration from details
   - Model selection via `TierRouter` + `PrivacyScanner`
   - Provider implementation hidden

10. **No Plugin System**
    - Features compiled in at build time
    - Configuration via `settings.json` only
    - No runtime code loading or script eval

11. **Embedding Outside Lock**
    - ONNX inference completes before acquiring SQLite Mutex
    - Applies to store, search, and consolidation

12. **Explicit Scope Writes**
    - Every `store_scoped()` requires explicit `WriteScope`
    - Never infer scope from file path alone
    - WriteScope param is mandatory, non-optional

---

## 11. Public API Surface

### `gaviero-core` (lib.rs)

```rust
// Types
pub use types::*;
pub use observer::{WriteGateObserver, AcpObserver, SwarmObserver};
pub use swarm::models::{WorkUnit, CompiledPlan, SwarmResult};
pub use memory::{MemoryStore, MemoryScope, WriteScope, SearchConfig};

// Tree-sitter (re-exported)
pub use tree_sitter::{LANGUAGE_REGISTRY, Language};

// Subsystems
pub mod swarm;        // pipeline::execute(), coordinator::plan()
pub mod acp;          // AcpPipeline, AcpSessionFactory
pub mod memory;       // MemoryStore methods
pub mod write_gate;   // WriteGatePipeline
pub mod workspace;    // Workspace::open()
pub mod git;          // GitRepo, WorktreeManager
pub mod iteration;    // IterationEngine
pub mod validation_gate; // ValidationPipeline
pub mod observer;      // Trait definitions
```

### `gaviero-dsl` (lib.rs)

```rust
pub fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>
```

### `gaviero-tui` (main.rs)

Executable. No public crate API.

### `gaviero-cli` (main.rs)

Executable. CLI argument parsing + swarm orchestration.

---

## 12. Key Dependencies

### Runtime

- **tokio 1.x:** async runtime, channels, sync primitives
- **tree-sitter 0.25:** 16 language grammars, AST parsing
- **git2 0.19:** git worktree, merge operations
- **rusqlite 0.32:** SQLite with bundled C library
- **sqlite-vec:** vector storage + similarity search
- **ort 2.0:** ONNX inference (embeddings)
- **petgraph 0.8:** DAG operations, Kahn's sort
- **ropey:** rope-based text buffer (editor)
- **portable-pty:** PTY spawning (embedded terminal)

### Parsing

- **logos:** lexer generation (DSL)
- **chumsky:** parser combinator (DSL)
- **miette:** error diagnostics with spans (DSL)
- **serde/serde_json:** serialization (session state, work units)

### UI

- **ratatui 0.30:** terminal rendering (TUI)
- **crossterm 0.29:** terminal I/O (TUI)
- **notify:** filesystem watcher (TUI)
- **arboard:** clipboard (TUI)

---

## 13. Design Principles

1. **Separation of Concerns:** Core (runtime), TUI (render/input), CLI (args), DSL (compile)
2. **Provider Abstraction:** Single backend trait, multiple implementations
3. **Scope Safety:** All writes checked against FileScope
4. **Lock Discipline:** Brief, focused mutexes; no lock across I/O
5. **Observable:** Callback observers for all significant events
6. **Testable:** Core runs without UI; backend implementations swappable
7. **Robust:** Checkpoints enable resumption; graceful degradation (e.g., no memory)
8. **Transparent:** Source spans preserved through DSL compilation; error diagnostics precise

---

See [CLAUDE.md](CLAUDE.md) for implementation conventions, build instructions, and testing.
