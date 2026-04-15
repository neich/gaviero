# gaviero-core — Architecture

Shared execution layer. All runtime logic: swarm orchestration, ACP chat, memory, write gate, validation, git, terminal. No UI dependencies.

---

## 1. Module Topology

```
gaviero-core/src/
├─ lib.rs                     Re-exports tree-sitter + 18 public modules
├─ types.rs                   Core types: FileScope, WriteProposal, ModelTier, PrivacyLevel
├─ workspace.rs               Workspace model, settings cascade
├─ session_state.rs           SessionState (UI tabs, panels, conversations)
├─ tree_sitter.rs             LANGUAGE_REGISTRY (16 langs), enrich_hunks()
├─ diff_engine.rs             compute_hunks() → Vec<DiffHunk>
├─ write_gate.rs              WriteGatePipeline, WriteMode, proposal lifecycle
├─ observer.rs                Observer trait definitions (WriteGateObserver, AcpObserver, SwarmObserver)
├─ scope_enforcer.rs          FileScope validation, permission checks
├─ git.rs                     GitRepo, WorktreeManager (git2 wrapper)
├─ query_loader.rs            Tree-sitter .scm query file discovery
├─ acp/                        Claude subprocess protocol & session management
│  ├─ session.rs              AcpSession, spawn, polling, stderr output
│  ├─ protocol.rs             NDJSON parsing (SystemInit, ContentDelta, ToolUseStart, etc.)
│  ├─ client.rs               AcpPipeline, prompt enrichment, file block routing
│  └─ factory.rs              AcpSessionFactory, session lifecycle
├─ swarm/                      Multi-agent orchestration engine (6-phase pipeline)
│  ├─ models.rs               WorkUnit, AgentManifest, SwarmResult, MergeResult
│  ├─ plan.rs                 CompiledPlan, PlanNode, petgraph DAG
│  ├─ pipeline.rs             Main orchestration: validate → execute → merge → verify → cleanup → consolidate
│  ├─ coordinator.rs          Natural-language task decomposition → TaskDAG (Opus-powered)
│  ├─ validation.rs           Scope overlap detection, Kahn's topological sort
│  ├─ router.rs               TierRouter: (ModelTier, PrivacyLevel) → ResolvedBackend
│  ├─ privacy.rs              PrivacyScanner: glob-based privacy override
│  ├─ calibration.rs          TierStats, per-tier success tracking
│  ├─ replanner.rs            Mid-execution replanning decisions
│  ├─ execution_state.rs      ExecutionState, NodeStatus, checkpoint/resume
│  ├─ merge.rs                Git merge + Claude-powered conflict resolution
│  ├─ bus.rs                  AgentBus: broadcast + targeted messaging
│  ├─ board.rs                SharedBoard: agent discovery board
│  ├─ context.rs              RepositoryContext collection + ref extraction
│  ├─ backend/                AgentBackend abstraction + implementations
│  │  ├─ mod.rs               AgentBackend trait, UnifiedStreamEvent, CompletionRequest, Capabilities
│  │  ├─ shared.rs            Model-spec parsing, provider detection, prompt enrichment
│  │  ├─ executor.rs          run_backend(), stream event processing
│  │  ├─ claude_code.rs       ClaudeCodeBackend (ACP subprocess via ClaudeCodeSession)
│  │  ├─ ollama.rs            OllamaStreamBackend (HTTP SSE)
│  │  ├─ codex.rs             CodexBackend (OpenAI-compatible)
│  │  ├─ mock.rs              MockBackend (testing)
│  │  └─ runner.rs            run_backend() orchestrator logic
│  └─ verify/                  Verification strategies
│     ├─ structural.rs         TreeSitterGate: parse-error detection
│     ├─ diff_review.rs        LLM diff review (batched per-unit/tier/aggregate)
│     ├─ test_runner.rs        Test command execution (auto-detect + targeted)
│     └─ combined.rs           Verification orchestrator (sequential + escalation)
├─ memory/                     Hierarchical scoped embeddings + consolidation
│  ├─ store.rs                MemoryStore (SQLite + sqlite-vec wrapper)
│  ├─ scope.rs                MemoryScope (5-level), WriteScope, Trust, MemoryType
│  ├─ scoring.rs              SearchConfig, ScoredMemory, scoring formula
│  ├─ consolidation.rs        Consolidator (3-phase: triage → decay → promotion)
│  ├─ embedder.rs             Embedder trait interface
│  ├─ onnx_embedder.rs        OnnxEmbedder (ort + tokenizers, nomic-embed-text-v1.5)
│  ├─ model_manager.rs        Model download + cache management
│  └─ code_graph.rs           Code knowledge graph (SQLite-backed)
├─ repo_map/                   File ranking + context graph
│  ├─ mod.rs                  RepoMap, build(), rank_for_agent()
│  ├─ store.rs                GraphStore, FileNode, DirectedEdge
│  └─ pagerank.rs             Personalized PageRank
├─ iteration/                  Retry + escalation + best-of-N engine
│  ├─ mod.rs                  IterationEngine, IterationConfig, Strategy
│  ├─ engine.rs               Retry loop logic, convergence detection
│  └─ converter.rs            TestGenerator: TDD test generation
├─ validation_gate/            Syntax + semantic validation gates
│  ├─ mod.rs                  ValidationPipeline, ValidationGate trait
│  ├─ tree_sitter_gate.rs     Parse-error detection
│  └─ cargo_check_gate.rs     Rust compile checks
├─ indent/                     Tree-sitter + heuristic indentation
│  ├─ mod.rs                  compute_indent() dispatcher
│  ├─ tree_sitter.rs          Query-based indent
│  ├─ hybrid.rs               Heuristic fallback
│  └─ bracket.rs              Bracket counting final fallback
└─ terminal/                   Embedded PTY + shell session management
   ├─ mod.rs                  TerminalManager, TerminalInstance lifecycle
   ├─ pty.rs                  PTY spawning (portable-pty)
   └─ osc133.rs               OSC 133 prompt/command parsing
```

---

## 2. Core Data Structures

### FileScope (types.rs:124)

Agent permission boundary.

```rust
pub struct FileScope {
    pub owned_paths: Vec<String>,
    pub read_only_paths: Vec<String>,
    pub interface_contracts: HashMap<String, String>,
}
```

**Invariants:**
- No two agents can own overlapping paths
- All file writes checked against this before accepting

**Used by:**
- `write_gate`: scope validation
- `swarm::validation`: overlap detection
- `memory/scope`: module-level namespace routing

### WorkUnit (swarm/models.rs:52)

Complete specification of one agent's task.

```rust
pub struct WorkUnit {
    pub id: String,
    pub description: String,
    pub scope: FileScope,
    pub depends_on: Vec<String>,
    pub coordinator_instructions: String,
    pub model: Option<String>,
    pub tier: ModelTier,
    pub privacy: PrivacyLevel,
    pub max_retries: u8,
    pub escalation_tier: Option<ModelTier>,
    // Memory routing fields
    pub read_namespaces: Option<Vec<String>>,
    pub write_namespace: Option<String>,
    pub memory_importance: Option<f32>,
    // Context expansion fields
    pub impact_scope: bool,
    pub context_callers_of: Vec<String>,
    pub context_tests_for: Vec<String>,
    pub context_depth: u32,
}
```

### CompiledPlan (swarm/plan.rs:18)

Immutable DAG of WorkUnits with metadata.

```rust
pub struct CompiledPlan {
    pub graph: DiGraph<PlanNode, DependencyEdge>,
    pub max_parallel: Option<usize>,
    pub source_file: Option<PathBuf>,
    pub iteration_config: IterationConfig,
    pub verification_config: VerificationConfig,
    pub loop_configs: Vec<LoopConfig>,
}
```

**Key methods:**
- `work_units_ordered() -> Vec<WorkUnit>`: Kahn's topo-sort
- `from_work_units(Vec<WorkUnit>) -> Result<Self>`: flat list → DAG with dependency edges
- `hash() -> String`: stable checkpoint identifier

### UnifiedStreamEvent (swarm/backend/mod.rs:84)

Normalized backend event stream (provider-agnostic).

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

**Produced by all backends; consumed by:**
- `backend::executor::complete_to_text()`: collects text
- `backend::executor::complete_to_write_gate()`: routes file blocks
- `acp/client.rs`: inserts proposals

### MemoryScope (memory/scope.rs:24)

Resolved scope chain for hierarchical search.

```rust
pub struct MemoryScope {
    pub global_db: PathBuf,
    pub workspace_db: Option<PathBuf>,
    pub workspace_id: Option<String>,
    pub repo_id: Option<String>,
    pub module_path: Option<String>,
    pub run_id: Option<String>,
}
```

**Search cascade:** run → module → repo → workspace → global (narrows → widens).

---

## 3. Subsystem: Swarm Pipeline

6-phase agent orchestration engine. Located in `swarm/pipeline.rs`.

### Phase 1: Validate

```rust
validate_scopes()              // O(n²) pairwise check: no overlap
work_units_ordered()           // Kahn's topo-sort
dependency_tiers()             // Vec<Vec<WorkUnit>> parallel groups
```

Aborts if validation fails.

### Phase 2: Execute

```
For each tier (sequential):
  For each unit (parallel, bounded by semaphore):
    ├─ git worktree checkout (gaviero/{unit_id})
    ├─ IterationEngine::run(unit, config)
    │  ├─ [if test_first] generate failing tests
    │  └─ FOR attempt in 0..max_attempts:
    │      ├─ escalate tier if attempt >= escalate_after
    │      ├─ run_backend(unit, tier, attempt)
    │      │  ├─ build_prompt() [memory, context, feedback]
    │      │  ├─ backend.stream_completion()
    │      │  ├─ FOR each FileBlock: write_gate.insert_proposal()
    │      │  └─ ValidationPipeline::run(modified_files)
    │      │      → PASS: next unit
    │      │      → FAIL: corrective prompt → retry
    │      └─ Checkpoint ExecutionState
```

### Phase 3: Merge

```
For each successful worktree:
  git merge --no-ff main
  on conflict: MergeResolver queries Claude for resolution
```

### Phase 4: Verify

```
Per VerificationStrategy:
  ├─ StructuralOnly: tree-sitter parse check
  ├─ DiffReview: LLM review (batched)
  ├─ TestSuite: run_tests
  └─ Combined: all three, sequential, early exit
Escalate on failure
```

### Phase 5: Cleanup

```
WorktreeManager::teardown_all()
Delete gaviero/* branches
```

### Phase 6: Consolidation

```
Consolidator::consolidate_run(run_id, repo_id)
  ├─ Triage: promote high-importance to module/repo
  ├─ Decay: exponential importance decay (30-day half-life)
  └─ Promotion: cross-scope if 3+ hits
```

---

## 4. Subsystem: Memory

Hierarchical, scoped embeddings with SQL backend and consolidation.

### Scope Hierarchy

```
0: Global        ~/.config/gaviero/memory.db
 └─ 1: Workspace <workspace>/.gaviero/memory.db
     └─ 2: Repo   Single git repo
         └─ 3: Module Crate/subdir (FileScope.owned_paths)
             └─ 4: Run   Ephemeral per-execution
```

### Search Pipeline (`memory/store.rs::search_scoped()`)

```
1. NO LOCK: embedder.embed(query_text) → Vec<f32>  [CPU: ONNX]

2. CASCADE (narrowest → widest):
   FOR level in scope.levels():
     │
     ├─ BRIEF LOCK: vec_search_at_level()
     │   SELECT from vec_memories_scoped WHERE scope_level = ?
     │   Release lock
     │
     ├─ BRIEF LOCK: fts_search_at_level()  [optional]
     │   SELECT from memories_fts INNER JOIN memories
     │   Release lock
     │
     ├─ NO LOCK: merge_rrf(vec_results, fts_results)
     │   RRF: 70% vector + 30% FTS
     │
     ├─ NO LOCK: score each candidate
     │   final = (sim*0.50 + importance*0.20 + recency*0.15 + 0.15)
     │           * scope_weight * trust_weight
     │
     └─ EARLY EXIT: if best_score > 0.70 stop widening

3. NO LOCK: deduplicate by content_hash

4. Return top-K ScoredMemory
```

**Lock discipline:** Embeddings outside lock. Brief lock per DB query. Scoring outside lock.

### Write Pipeline (`memory/store.rs::store_scoped()`)

```
1. NO LOCK: embedder.embed(content) → Vec<f32>

2. NO LOCK: content_hash = SHA-256(normalized(content))

3. BRIEF LOCK: deduplication check
   SELECT FROM memories WHERE content_hash = ?
   ├─ Exact match same scope → reinforce (update importance)
   │   → StoreResult::Deduplicated(id)
   ├─ Exact match broader scope → skip
   │   → StoreResult::AlreadyCovered
   └─ No match → INSERT
      → StoreResult::Inserted(id)

4. BRIEF LOCK: insert into vec_memories_scoped + memories_fts
   (FTS trigger auto-updates search index)
```

### Consolidation (`memory/consolidation.rs::Consolidator`)

3-phase pipeline, runs after swarm execution:

```
Phase 1: RUN TRIAGE
  query_by_run(run_id) → Vec<ScoredMemory>
  FOR each with importance >= 0.4:
    store_scoped(Module or Repo, content, consolidation_meta)
  delete_by_run(run_id)

Phase 2: DECAY + PRUNING
  FOR each memory:
    importance *= exp(-0.023 * days_since_access)
    if importance < threshold: delete

Phase 3: CROSS-SCOPE PROMOTION
  find_promotion_candidates(min_cross_hits=3)
  → memories accessed by 3+ different modules
  promote to Repo scope, 1.2x boost
```

### Embedder (`memory/embedder.rs`)

**Interface:**

```rust
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
    fn model_id(&self) -> &str;
}
```

**Implementation: OnnxEmbedder** (`onnx_embedder.rs`)
- **Model:** nomic-embed-text-v1.5 (768 dimensions)
- **Runtime:** `ort` (ONNX) + `tokenizers` crate
- **Pooling:** mean pooling + L2 norm
- **Vector search:** sqlite-vec `vec0` virtual table, cosine distance

---

## 5. Subsystem: Backend Abstraction

Provider-agnostic interface. Located in `swarm/backend/`.

### AgentBackend Trait (mod.rs:45)

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
```

### Implementations

| Backend | Code | Provider | Protocol |
|---|---|---|---|
| ClaudeCode | `claude_code.rs` | Claude API | ACP subprocess (NDJSON) |
| Ollama | `ollama.rs` | Local Ollama | HTTP SSE `/api/generate` |
| Codex | `codex.rs` | OpenAI-compatible | OpenAI stream format |
| Mock | `mock.rs` | Synthetic (testing) | In-memory event queue |

### Provider Resolution (`swarm/backend/shared.rs`)

**Model-spec parsing:**
```
"claude:opus"          → Claude Opus (via ACP)
"claude:sonnet"        → Claude Sonnet
"ollama:llama2"        → Ollama + model name
"local:endpoint"       → OpenAI-compatible endpoint
"sonnet"               → Default Claude (Sonnet)
```

**Tier routing (`swarm/router.rs::TierRouter`):**
```
(ModelTier, PrivacyLevel) → ResolvedBackend

Cheap + Public      → Claude Haiku
Expensive + Public  → Claude Sonnet/Opus
LocalOnly          → Ollama (if enabled) or Blocked
```

**Privacy override (`swarm/privacy.rs::PrivacyScanner`):**
- Glob-based path matching (e.g., `**/*.key`, `**/.env`)
- Overrides privacy to `LocalOnly` even if unit specifies Public

---

## 6. Subsystem: Write Gate

Enforces file-write safety. Located in `write_gate.rs`.

### WriteMode Enum

| Mode | Behavior | When used |
|---|---|---|
| `Interactive` | Queue proposals → TUI review | Editor (chat) |
| `AutoAccept` | Validate scope, accept + write | Swarm (CI-friendly) |
| `Deferred` | Accumulate for batch review | TUI agent chat |
| `RejectAll` | Silently discard | Safety fallback |

### Proposal Lifecycle

```
WorkUnit.write(path, content)
│
├─ 1. BRIEF LOCK: is_scope_allowed(agent_id, path)?
│    Release lock
│    ├─ PASS → continue
│    └─ FAIL → reject, observer callback
│
├─ 2. NO LOCK: compute diff
│    original = fs::read_to_string(path)
│    hunks = diff_engine::compute_hunks(original, content)
│    structural = tree_sitter::enrich_hunks(hunks, original, lang)
│
├─ 3. BRIEF LOCK: insert_proposal()
│    ├─ Interactive → queue + on_proposal_created()
│    ├─ AutoAccept → accept all + finalize
│    ├─ Deferred → accumulate
│    └─ RejectAll → discard
│    Release lock
│
└─ 4. [if finalized] NO LOCK: fs::write(path, content)
```

**Lock discipline:** Mutex held for O(1) operations only. All I/O, parsing, diff outside lock.

---

## 7. Subsystem: ACP (Claude Subprocess)

Claude Code integration. Located in `acp/`.

### Session Lifecycle

```
AcpSessionFactory
├─ one_shot(model, prompt) → Result<String>
│   One prompt, one response, cleanup
└─ persistent(model) → AcpSession
   persistent.send_prompt(prompt) → Stream<AcpEvent>
   persistent.next_event() → Option<AcpEvent>
```

### NDJSON Protocol (protocol.rs)

**Event types:**

| Type | JSON | Meaning |
|---|---|---|
| `SystemInit` | `{ session_id, model }` | Session established |
| `ContentDelta` | `{ delta: { type: "text_delta", text: "..." } }` | Streaming text |
| `ToolUseStart` | `{ delta: { type: "input_json_delta", input: "..." } }` | Tool call starting |
| `AssistantMessage` | `{ message: { content: [...] } }` | Turn complete |
| `ResultEvent` | `{ event: "result", ... }` | Final result with cost |

### AcpPipeline (client.rs)

Prompt enrichment + file block routing:

```rust
pub struct AcpPipeline {
    session: AcpSession,
    write_gate: Arc<Mutex<WriteGatePipeline>>,
}

impl AcpPipeline {
    pub async fn propose_write(&self, path: PathBuf, content: String) -> Result<()> {
        // File block detected in stream
        // Scope check → diff → structural enrichment → write_gate.insert_proposal()
    }
}
```

---

## 8. Concurrency Model

Single shared `tokio` runtime.

### Sync Primitives Used

```rust
Arc<tokio::sync::Mutex<T>>         // WriteGatePipeline, MemoryStore
Arc<dyn Observer>                  // Observer trait objects
mpsc::UnboundedChannel<Event>      // TUI event loop (single receiver)
broadcast::channel                 // AgentBus (multi-receiver)
Semaphore                          // Parallel agent count bound
```

### Lock Rules

| Component | Lock Type | Golden Rule |
|---|---|---|
| WriteGatePipeline | Mutex | Never hold across I/O, parsing, or diff computation |
| MemoryStore | Mutex<rusqlite::Connection> | Embedding outside lock. Brief lock per DB op. |
| ExecutionState | Mutex<Vec<NodeStatus>> | Checkpoint after each node (resumable) |
| AgentBus | broadcast::channel | Lock-free, async-friendly |

**Never hold Mutex across await, tree-sitter parse, or fs I/O.**

---

## 9. Error Handling Strategy

| Error | Module | Recoverable? | Handling |
|---|---|---|---|
| Parse/compile error | dsl, validator | Time-of-compile | Miette diagnostic return |
| Scope violation | write_gate | Runtime | Reject proposal, log |
| Agent execution failure | swarm::backend | Yes | AgentStatus::Failed, escalate if configured |
| Validation failure | validation_gate | Yes | Corrective feedback → retry same agent |
| Merge conflict | merge | Interactive | Claude resolution or user choice |
| Memory init failure | memory | Non-fatal | Option<Arc<MemoryStore>>, continue without memory |
| Consolidation failure | consolidation | Non-fatal | Log, continue (best-effort) |

---

## 10. Hard Constraints

Architectural invariants. Do not violate.

1. **Write Gate Mandatory**
   - All agent writes through `WriteGatePipeline`
   - No direct `fs::write` from agent paths
   - Scope validation before any disk I/O

2. **git2 Only**
   - Never shell out to `git`
   - All git ops via `git2` crate

3. **Tree-Sitter for Structure**
   - Syntax analysis: tree-sitter queries
   - Highlighting: tree-sitter queries
   - Indentation: tree-sitter + heuristic + bracket fallback
   - 16-language registry

4. **Mutex-Wrapped SQLite**
   - `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`
   - All DB methods async/awaitable
   - Embedding outside lock

5. **Observer-Only Coupling**
   - Core never imports TUI/CLI types
   - All communication via trait objects
   - Backends opaque to orchestration

6. **Provider-Agnostic Backend**
   - `AgentBackend` + `UnifiedStreamEvent`
   - Model selection via `TierRouter` + `PrivacyScanner`

7. **Embedding Outside Lock**
   - ONNX inference completes before SQLite Mutex
   - Applies to store, search, consolidation

8. **Explicit Scope Writes**
   - Every `store_scoped()` requires explicit `WriteScope`
   - Never infer scope from path
   - `WriteScope` parameter mandatory, non-optional

---

## 11. Public API

```rust
// Root re-exports
pub use types::*;
pub use observer::{WriteGateObserver, AcpObserver, SwarmObserver};
pub use swarm::models::{WorkUnit, CompiledPlan, SwarmResult};
pub use memory::{MemoryStore, MemoryScope, WriteScope, SearchConfig};
pub use tree_sitter::LANGUAGE_REGISTRY;

// Subsystem modules
pub mod swarm;              // pipeline::execute(), coordinator::plan()
pub mod acp;                // AcpPipeline, AcpSessionFactory
pub mod memory;             // MemoryStore methods
pub mod write_gate;         // WriteGatePipeline
pub mod workspace;          // Workspace::open()
pub mod git;                // GitRepo, WorktreeManager
pub mod iteration;          // IterationEngine
pub mod validation_gate;    // ValidationPipeline
pub mod observer;           // Trait definitions
pub mod repo_map;           // RepoMap
```

---

## 12. Key Dependencies

- **tokio 1.x:** async runtime, channels
- **tree-sitter 0.25:** 16 grammars
- **git2 0.19:** worktrees, merge
- **rusqlite 0.32 + sqlite-vec:** vector storage
- **ort 2.0:** ONNX inference
- **petgraph 0.8:** DAG ops
- **ropey, portable-pty, vt100:** text buffer, PTY, terminal

---

See [CLAUDE.md](CLAUDE.md) for conventions, build, test instructions.
