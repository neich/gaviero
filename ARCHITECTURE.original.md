# Gaviero — Architecture

> Terminal code editor for AI agent orchestration, written in Rust 2024.

**Binaries:** `gaviero` (TUI editor), `gaviero-cli` (headless swarm runner)
**Platform:** Linux (primary), any POSIX with a modern terminal
**Build:** `cargo build` from workspace root — no external tooling

---

## 1. Crate Topology

```
                 ┌──────────────┐     ┌──────────────┐
                 │  gaviero-tui │     │  gaviero-cli │
                 │  (binary)    │     │  (binary)    │
                 └──────┬───────┘     └──────┬───────┘
                        │                    │
                        ├────────┬───────────┤
                        ▼        ▼           ▼
                 ┌────────────┐  ┌────────────────┐
                 │gaviero-core│  │  gaviero-dsl   │
                 │  (library) │  │  (library)     │
                 └──────┬─────┘  └───────┬────────┘
                        │                │
                        │◄───────────────┘  (dsl depends on core)
                        │
                 ┌──────┴──────────────┐
                 │ tree-sitter-gaviero │
                 │ (grammar crate)     │
                 └─────────────────────┘
```

**Separation rule:** All pipeline logic lives in `gaviero-core`. The TUI crate contains only rendering and input handling. The CLI crate contains only argument parsing and observer implementations. The DSL crate contains only `.gaviero` language compilation. Core can be tested without any UI dependency.

| Crate | Role | Key dependencies |
|---|---|---|
| `gaviero-core` | Logic: write gate, diff, tree-sitter, ACP, swarm, memory, git, terminal, repo map, iteration, validation | tokio, tree-sitter 0.25 + 16 langs, git2, rusqlite + sqlite-vec, ort, petgraph, similar, ropey, portable-pty, vt100, reqwest, async-trait, futures, sha2 |
| `gaviero-tui` | TUI editor binary | gaviero-core, gaviero-dsl, ratatui 0.30, crossterm 0.29, notify, arboard, tui-term |
| `gaviero-cli` | Headless swarm runner | gaviero-core, gaviero-dsl, clap, tokio |
| `gaviero-dsl` | `.gaviero` DSL compiler | gaviero-core, logos, chumsky, miette, thiserror |
| `tree-sitter-gaviero` | Tree-sitter grammar for `.gaviero` syntax highlighting | tree-sitter (build-time C grammar compilation) |

Tree-sitter types are re-exported from `gaviero-core::lib.rs` (`Language`, `Tree`, `Parser`, `Query`, `QueryCursor`, `InputEdit`, `Node`, `Point`). Downstream crates never depend on `tree-sitter` directly.

---

## 2. Module Map

### gaviero-core/src/ (18 public modules)

```
lib.rs                      Re-exports tree-sitter types; declares 18 pub modules
types.rs                    FileScope, DiffHunk, HunkType, WriteProposal, StructuralHunk, NodeInfo,
                            SymbolKind, ModelTier, PrivacyLevel, TierAnnotation, EntryMetadata
workspace.rs                Workspace model, WorkspaceFolder, settings cascade (cached)
session_state.rs            SessionState, TabState, PanelState, StoredConversation
tree_sitter.rs              LANGUAGE_REGISTRY (16 langs), enrich_hunks(), language_for/name_for_extension()
diff_engine.rs              compute_hunks() — similar crate wrapper → Vec<DiffHunk>
write_gate.rs               WriteGatePipeline, WriteMode, proposal management
observer.rs                 WriteGateObserver, AcpObserver, SwarmObserver trait definitions
scope_enforcer.rs           Path-level read/write permission checks + hardcoded block-list
git.rs                      GitRepo (git2 wrapper), WorktreeManager, GitCoordinator, FileStatus
query_loader.rs             Tree-sitter .scm file discovery (env var → exe dir → cwd → bundled)
indent/                     compute_indent() entry point — tree-sitter, hybrid heuristic, bracket fallback
terminal/                   TerminalManager → TerminalInstance → PTY (portable-pty + vt100)
acp/                        AcpSession, AcpPipeline, AcpSessionFactory, NDJSON protocol
memory/                     MemoryStore (SQLite + sqlite-vec), OnnxEmbedder, CodeGraph, Consolidator,
                            MemoryScope (5-level hierarchy), SearchConfig, ScoredMemory
repo_map/                   RepoMap, FileNode, ContextPlan — PageRank-based context + GraphStore
iteration/                  IterationEngine, Strategy, IterationConfig, ConvergenceDetector, TestGenerator
validation_gate/            ValidationPipeline, ValidationGate trait, TreeSitterGate, CargoCheckGate
swarm/                      Pipeline, Coordinator, TierRouter, AgentBus, SharedBoard, verify/, backend/
```

### gaviero-tui/src/

```
main.rs                     Entry point, terminal setup, event loop, panic handler
app.rs                      App state, layout rendering, focus management, find bar (~5000 lines)
event.rs                    Event enum (43+ variants), EventLoop (crossterm/watcher/tick/terminal bridge)
keymap.rs                   KeyEvent → Action mapping; chord-prefix support
theme.rs                    ~80 colour constants (One Dark), timing constants

editor/
  buffer.rs                 Ropey buffer, Cursor, Transaction, undo/redo, find_next/prev_match
  view.rs                   EditorView widget: gutter, syntax highlights, cursor, scrollbar
  diff_overlay.rs           Diff review mode: DiffSource, DiffReviewState, accept/reject per hunk
  highlight.rs              Tree-sitter highlight query runner → Vec<StyledSpan>
  markdown.rs               Markdown document rendering and editing

panels/
  file_tree.rs              Multi-root file browser, git + proposal decorations
  agent_chat.rs             AgentChatState, Conversation, attachments, @file autocomplete
  chat_markdown.rs          ChatLine: markdown rendering for chat messages
  swarm_dashboard.rs        Agent status table with tier/phase labels
  git_panel.rs              GitPanelState, staging area, commit, branch picker
  terminal.rs               Terminal rendering (tui-term), TerminalSelectionState
  status_bar.rs             Mode, file, branch, agent status indicators
  search.rs                 SearchPanelState, interactive input + live results

widgets/
  tabs.rs                   TabBar widget with close indicators
  scrollbar.rs              Custom scrollbar widget
  scroll_state.rs           ScrollState: shared scroll offset + selection for list panels
  text_input.rs             TextInput: shared text editing with cursor, selection, undo/redo
  render_utils.rs           Shared rendering utilities
```

### gaviero-cli/src/

```
main.rs                     Cli struct (clap derive), CliAcpObserver, CliSwarmObserver, pipeline launcher
```

### gaviero-dsl/src/

```
lib.rs                      pub fn compile(source, filename, workflow, runtime_prompt) → Result<CompiledPlan>
lexer.rs                    Token enum (logos derive), lex() function
ast.rs                      Script, Item, ClientDecl, AgentDecl, WorkflowDecl, ContextBlock, ScopeBlock,
                            MemoryBlock, VerifyBlock, LoopBlock, UntilCondition, StrategyLit, TierLit
parser.rs                   parse() — chumsky combinators; grammar defined as functions
compiler.rs                 compile_ast() — 7-phase semantic analysis
error.rs                    DslError (Lex/Parse/Compile variants), DslErrors (miette wrapper)
```

---

## 3. Core Abstractions

### FileScope (`types.rs`)

Defines an agent's permission boundary over the filesystem.

```
FileScope {
    owned_paths: Vec<String>                   Files/dirs the agent may write
    read_only_paths: Vec<String>               Files/dirs the agent may read only
    interface_contracts: HashMap<String,String> API contracts the agent must preserve
}
```

Used by: Write Gate (scope validation), Swarm (overlap detection), Agent Runner (prompt enrichment), Memory (module path derivation).

### WorkUnit (`swarm/models.rs`)

A single task assigned to one swarm agent.

```
WorkUnit {
    id: String                                 Unique identifier
    description: String                        Task description
    scope: FileScope                           Read/write boundaries
    depends_on: Vec<String>                    Prerequisite WorkUnit IDs
    backend: AgentBackend                      ClaudeCode | Ollama | Custom (deprecated enum)
    model: Option<String>                      Per-unit model override
    tier: ModelTier                             Cheap | Expensive
    privacy: PrivacyLevel                      Public | LocalOnly
    coordinator_instructions: String           Decomposed subtask instructions
    max_retries: u8                             Max retries before escalation
    escalation_tier: Option<ModelTier>         Tier to escalate to on failure

    // Memory routing
    read_namespaces: Option<Vec<String>>       Memory namespaces to read
    write_namespace: Option<String>            Memory namespace to write to
    memory_importance: Option<f32>             Importance weight (0.0–1.0)
    staleness_sources: Vec<String>             Paths for staleness checks
    memory_read_query: Option<String>          Custom memory search query
    memory_read_limit: Option<usize>           Custom result limit
    memory_write_content: Option<String>       Template for memory writes

    // Graph / impact fields
    impact_scope: bool                         Auto-expand read_only with blast-radius
    context_callers_of: Vec<String>            Files for caller graph queries
    context_tests_for: Vec<String>             Paths for test file queries
    context_depth: u32                          BFS depth for graph queries (default: 2)
}
```

### CompiledPlan (`swarm/plan.rs`)

Immutable execution plan — the output of `gaviero_dsl::compile()` and input to `swarm::pipeline::execute()`.

```
CompiledPlan {
    graph: DiGraph<PlanNode, DependencyEdge>   petgraph DAG
    max_parallel: Option<usize>                 Concurrency cap
    source_file: Option<PathBuf>               Source .gaviero file
    iteration_config: IterationConfig          Strategy + retry + escalation config
    verification_config: VerificationConfig    compile/clippy/test/impact_tests flags
    loop_configs: Vec<LoopConfig>              Explicit loop configurations
}
```

Key methods: `work_units_ordered()` (Kahn's topological sort), `from_work_units()` (flat list → DAG), `hash()` (stable checkpoint naming).

### WriteProposal (`types.rs`)

```
WriteProposal {
    id: u64                           Unique proposal ID
    source: String                    Agent ID or "user"
    file_path: PathBuf                Target file
    original_content: String          Before
    proposed_content: String          After
    structural_hunks: Vec<StructuralHunk>   DiffHunk + AST context
    status: ProposalStatus            Pending | PartiallyAccepted | Accepted | Rejected
}
```

### AgentBackend trait (`swarm/backend/mod.rs`)

```rust
#[async_trait]
trait AgentBackend: Send + Sync {
    async fn stream_completion(&self, req: CompletionRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;
    fn capabilities(&self) -> Capabilities;
    fn name(&self) -> &str;
    async fn health_check(&self) -> Result<()>;
}
```

`UnifiedStreamEvent` variants: `TextDelta`, `ThinkingDelta`, `ToolCallStart`, `ToolCallDelta`, `ToolCallEnd`, `FileBlock`, `Usage`, `Error`, `Done`.

Implementations: `ClaudeCodeBackend` (ACP subprocess), `OllamaStreamBackend` (HTTP SSE), `MockBackend` (deterministic test fixture).

### Observer traits (`observer.rs`)

```
WriteGateObserver — on_proposal_created, on_proposal_updated, on_proposal_finalized
AcpObserver      — on_stream_chunk, on_tool_call_started, on_streaming_status,
                   on_message_complete, on_proposal_deferred, on_permission_request,
                   on_validation_result, on_validation_retry
SwarmObserver    — on_phase_changed, on_agent_state_changed, on_tier_started,
                   on_merge_conflict, on_completed, on_coordination_started,
                   on_coordination_complete, on_tier_dispatch, on_escalation,
                   on_verification_started/step_started/step_complete/complete,
                   on_cost_update
```

---

## 4. Event Architecture (TUI)

A single `tokio::sync::mpsc::unbounded_channel<Event>` carries all external events to the TUI main loop. No background task mutates application state directly.

### Event Sources

| Source | Events produced | Mechanism |
|---|---|---|
| Crossterm reader | `Key`, `Mouse`, `Paste`, `Resize` | Blocking thread → channel |
| File watcher (notify) | `FileChanged`, `FileTreeChanged` | Callback → channel |
| Tick timer | `Tick` (~33ms, ~30fps) | tokio::interval → channel |
| Terminal bridge | `Terminal(TerminalEvent)` | TerminalManager mpsc → event channel |
| WriteGateObserver | `ProposalCreated`, `ProposalUpdated`, `ProposalFinalized` | Observer trait impl |
| AcpObserver | `StreamChunk`, `ToolCallStarted`, `StreamingStatus`, `MessageComplete`, `FileProposalDeferred`, `AcpTaskCompleted` | Observer trait impl |
| SwarmObserver | `SwarmPhaseChanged`, `SwarmAgentStateChanged`, `SwarmTierStarted`, `SwarmMergeConflict`, `SwarmCompleted`, `SwarmCoordinationStarted`, `SwarmCoordinationComplete`, `SwarmTierDispatch`, `SwarmCostUpdate`, `SwarmDslPlanReady` | Observer trait impl |
| Memory init | `MemoryReady` | Background spawn → channel |

### Observer Bridge Pattern

```
                    gaviero-core                         gaviero-tui
              ┌─────────────────────┐            ┌──────────────────────┐
              │  WriteGateObserver  │◄───────────│  TuiWriteGateObserver│
              │  AcpObserver        │◄───────────│  TuiAcpObserver      │
              │  SwarmObserver      │◄───────────│  TuiSwarmObserver    │
              └─────────────────────┘            └──────────┬───────────┘
                                                            │
                                                   sends Event to channel
                                                            │
                                                            ▼
                                                     App::handle_event()
```

### Main Loop

```
loop {
    terminal.draw(|frame| app.render(frame));
    event = event_rx.recv().await;
    app.handle_event(event);
    // drain up to 64 pending events before redrawing
    if app.should_quit { break; }
}
```

---

## 5. Data Flow: Agent Write Proposal

Every agent file write passes through this pipeline.

```
  Agent (AcpSession / OllamaStream)
    │
    │  <file path="src/foo.rs">...content...</file>   (detected in stream)
    │
    ▼
  AcpPipeline::propose_write(path, content)
    │
    ├─ 1. BRIEF LOCK: write_gate.is_scope_allowed(agent_id, path)?
    │     Release lock
    │
    ├─ 2. NO LOCK:
    │     original = fs::read_to_string(path)
    │     hunks = diff_engine::compute_hunks(original, content)
    │     structural = tree_sitter::enrich_hunks(hunks, original, language)
    │     proposal = WriteProposal { hunks: structural, status: Pending }
    │
    ├─ 3. BRIEF LOCK: write_gate.insert_proposal(proposal)
    │     ├─ Interactive → queue, fire on_proposal_created() → TUI shows diff overlay
    │     ├─ AutoAccept → accept all, return Some((path, content))
    │     ├─ Deferred → accumulate for batch review
    │     └─ RejectAll  → discard silently
    │     Release lock
    │
    └─ 4. NO LOCK: if AutoAccept, write content to disk
```

**Lock discipline:** The `WriteGatePipeline` Mutex is never held across I/O, tree-sitter parsing, or diff computation.

---

## 6. Data Flow: Swarm Execution

```
  swarm::pipeline::execute(plan, config, checkpoint, memory, observer, make_obs)
    │
    ├─ Phase 1: VALIDATE
    │   validate_scopes() → check no owned_path overlaps (O(n^2) pairwise)
    │   work_units_ordered() → Kahn's topological sort from plan graph
    │   dependency_tiers() → Vec<Vec<WorkUnit>> (parallel groups)
    │
    ├─ Phase 2: EXECUTE (per tier, sequentially)
    │   │
    │   │  Tier N: [A, B, C]  (can run in parallel)
    │   │
    │   ├─ For each WorkUnit (bounded by Semaphore):
    │   │   ├─ Provision git worktree (branch: gaviero/{id})
    │   │   ├─ IterationEngine::run()
    │   │   │   ├─ [test_first] TestGenerator::generate()
    │   │   │   └─ FOR attempt in 0..n_attempts:
    │   │   │       escalate model if attempt ≥ escalate_after
    │   │   │       └─► run_backend()
    │   │   │           ├─ build_prompt()
    │   │   │           │   scoped memory context (cascading search)
    │   │   │           │   file scope clause
    │   │   │           │   repo_map outline (PageRank-ranked)
    │   │   │           │   shared_board discoveries
    │   │   │           │   corrective feedback (on retry)
    │   │   │           ├─ backend.stream_completion(request)
    │   │   │           │   → Stream<UnifiedStreamEvent>
    │   │   │           ├─ FOR EACH FileBlock: write_gate.insert_proposal()
    │   │   │           └─ ValidationPipeline::run(modified_files)
    │   │   │               TreeSitterGate → CargoCheckGate
    │   │   │               PASS → done  |  FAIL → corrective → retry
    │   │   └─ Broadcast to AgentBus + post to SharedBoard
    │   │
    │   └─ Checkpoint ExecutionState to disk after each node
    │
    ├─ Phase 3: MERGE (if use_worktrees)
    │   For each successful agent branch:
    │     git merge --no-ff → main
    │     On conflict: MergeResolver queries Claude for resolution
    │
    ├─ Phase 4: VERIFY (optional, per VerificationStrategy)
    │   StructuralOnly | DiffReview | TestSuite | Combined
    │   Escalation on failure → re-run failed agents at higher tier
    │
    ├─ Phase 5: WORKTREE CLEANUP
    │   WorktreeManager::teardown_all()
    │
    ├─ Phase 6: MEMORY CONSOLIDATION (best-effort)
    │   Consolidator::consolidate_run(run_id, repo_id)
    │     Promotes durable run memories to module/repo scope
    │     Deletes ephemeral run-level memories
    │
    └─ Return SwarmResult { manifests, merge_results, success, pre_swarm_sha }
```

### Loop execution

When the plan contains `LoopConfig` entries, agents in the loop repeat until the `LoopUntilCondition` is met or `max_iterations` is reached. Conditions: `Verify` (compile/test pass), `Agent` (judge returns pass), `Command` (exit code 0).

---

## 7. Memory System

### Scope Hierarchy

Five levels from broadest to narrowest:

```
global (0)             personal cross-workspace knowledge (~/.config/gaviero/memory.db)
  └─ workspace (1)     business-level project (<workspace>/.gaviero/memory.db)
       └─ repo (2)     single git repository
            └─ module (3)  crate/package/subdirectory (FileScope.owned_paths)
                 └─ run (4)   single swarm execution (ephemeral, consolidated upward)
```

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                         MemoryStore                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐  ┌────────────┐  │
│  │ rusqlite │  │ Embedder │  │ Consolidator │  │  Scoring   │  │
│  │Connection│  │  (ONNX)  │  │ (3-phase)    │  │  (RRF)     │  │
│  │ (Mutex)  │  │          │  │              │  │            │  │
│  └──────────┘  └──────────┘  └──────────────┘  └────────────┘  │
│                                                                  │
│  Tables:                                                         │
│    memories (scope_level, scope_path, repo_id, module_path,     │
│              run_id, content_hash, memory_type, trust, tag)     │
│    vec_memories_scoped (embedding + scope_level partition)       │
│    memories_fts (FTS5 full-text, porter+unicode61 tokenizer)    │
│    memory_access_log (cross-scope promotion heuristics)         │
│    episodes (agent run tracking)                                 │
│    graph_state (code knowledge graph serialization)             │
└──────────────────────────────────────────────────────────────────┘
```

### Key Types

| Type | Module | Description |
|---|---|---|
| `MemoryScope` | `memory/scope.rs` | Resolved scope chain from execution context: global_db, workspace_db, workspace_id, repo_id, module_path, run_id |
| `WriteScope` | `memory/scope.rs` | Target scope for a write: `Global` \| `Workspace` \| `Repo{repo_id}` \| `Module{repo_id,module_path}` \| `Run{repo_id,run_id}` |
| `ScopeFilter` | `memory/scope.rs` | Filter for a single scope level during cascading search |
| `WriteMeta` | `memory/scope.rs` | Write metadata: memory_type, importance, trust, source, tag |
| `Trust` | `memory/scope.rs` | `High` (user /remember, weight 1.2) \| `Medium` (agent, weight 1.0) \| `Low` (consolidated, weight 0.7) |
| `MemoryType` | `memory/scope.rs` | `Factual` \| `Procedural` \| `Decision` \| `Pattern` \| `Gotcha` |
| `StoreResult` | `memory/scope.rs` | `Inserted(id)` \| `Deduplicated(id)` \| `AlreadyCovered` |
| `SearchConfig` | `memory/scoring.rs` | Query + max_results + per_level_limit + similarity/confidence thresholds + scope chain |
| `ScoredMemory` | `memory/scoring.rs` | Memory entry with raw_similarity, fts_rank, and final_score |

### Data Flow: Scoped Memory Search

```
  Caller (build_prompt / Coordinator / MergeResolver)
    │
    │  memory.search_scoped(config: &SearchConfig)
    │
    ├─ 1. NO LOCK: embedder.embed(query_text) → Vec<f32>   [CPU-heavy, ONNX inference]
    │
    ├─ 2. CASCADE through scope levels (narrowest → widest):
    │     FOR EACH level in scope.levels() [run → module → repo → workspace → global]:
    │       │
    │       ├─ BRIEF LOCK: vec_search_at_level(embedding, level, per_level_limit)
    │       │   SELECT from vec_memories_scoped WHERE scope_level = ?
    │       │   Release lock
    │       │
    │       ├─ [if use_fts] BRIEF LOCK: fts_search_at_level(query_text, level)
    │       │   SELECT from memories_fts INNER JOIN memories
    │       │   Release lock
    │       │
    │       ├─ NO LOCK: merge_rrf(vec_results, fts_results, k=60)
    │       │   Reciprocal Rank Fusion: vector 70% weight + FTS 30% weight
    │       │
    │       ├─ NO LOCK: score each candidate:
    │       │   final = (similarity*0.50 + importance*0.20 + recency*0.15 + 0.15)
    │       │           × scope_weight × trust_weight × access_reinforcement
    │       │
    │       └─ EARLY TERMINATION: if best_score > confidence_threshold (0.70)
    │          stop widening scope — narrow results are sufficient
    │
    ├─ 3. NO LOCK: deduplicate by content_hash across levels
    │
    └─ 4. Return top-K ScoredMemory entries, sorted by final_score desc
```

### Data Flow: Scoped Memory Write

```
  Caller (agent observation, user /remember, consolidation promotion)
    │
    │  memory.store_scoped(scope: &WriteScope, content, meta: &WriteMeta)
    │
    ├─ 1. NO LOCK: embedder.embed(content) → Vec<f32>
    │
    ├─ 2. NO LOCK: content_hash = SHA-256(normalized(content))
    │
    ├─ 3. BRIEF LOCK: check dedup
    │     SELECT FROM memories WHERE content_hash = ? AND scope_level <= ?
    │     ├─ Exact match at same scope → reinforce (update importance, access_count)
    │     │   → return StoreResult::Deduplicated(id)
    │     ├─ Exact match at broader scope → skip
    │     │   → return StoreResult::AlreadyCovered
    │     └─ No match → INSERT with scope metadata
    │        → return StoreResult::Inserted(id)
    │
    └─ 4. BRIEF LOCK: insert into vec_memories_scoped + memories_fts (via trigger)
```

### Consolidation Pipeline

Runs after each swarm execution (Phase 6 in pipeline):

```
Consolidator::consolidate_run(run_id, repo_id)
  │
  ├─ Phase 1: RUN TRIAGE
  │   query_by_run(run_id) → Vec<ScoredMemory>
  │   FOR EACH run memory with importance >= 0.4:
  │     store_scoped(Module or Repo scope, content, WriteMeta::consolidation())
  │     → Inserted | Deduplicated | AlreadyCovered
  │   delete_by_run(run_id) — remove all ephemeral run memories
  │
  ├─ Phase 2: IMPORTANCE DECAY + PRUNING
  │   decay_and_prune()
  │   Exponential decay: importance *= exp(-0.023 × days_since_access)
  │   Half-life: 30 days. Prune entries below threshold.
  │
  └─ Phase 3: CROSS-SCOPE PROMOTION
      find_promotion_candidates(min_cross_hits=3)
      Module memories accessed by agents in 3+ different modules
      → promote copy to Repo scope with 1.2× importance boost
```

### Schema Migrations

| Version | Changes |
|---|---|
| v1 | `memories` table, namespace/key indexes |
| v2 | `privacy` column for tier routing |
| v3 | `importance`, `access_count`, `source_file/hash`, `vec_memories` (sqlite-vec), `episodes`, `graph_state`. Nullified embeddings for model change (384d→768d). |
| v4 | Scope columns (`scope_level`, `scope_path`, `repo_id`, `module_path`, `run_id`, `content_hash`, `memory_type`, `trust`, `tag`). FTS5 index with sync triggers. `vec_memories_scoped` (scope-partitioned vectors). `memory_access_log`. Content hash backfill. Namespace→scope migration. |

### Embedder

- **Embedder trait:** `embed(text) → Vec<f32>`, `dimensions()`, `model_id()`
- **OnnxEmbedder:** `ort` (ONNX Runtime) + `tokenizers`, mean pooling, L2 normalization
- **Production model:** nomic-embed-text-v1.5 (768 dimensions)
- **Vector search:** sqlite-vec `vec0` virtual table with cosine distance

### Code Knowledge Graph (`memory/code_graph.rs`, `repo_map/store.rs`)

SQLite-backed directed graph of code structure:
- **Nodes:** `File`, `Function`, `Struct`, `Trait`, `Enum`, `Test` — with qualified names and file hashes
- **Edges:** `Imports`, `Calls`, `Implements`, `TestedBy`, `Contains`
- **Incremental builds:** `graph_builder.rs` compares file hashes, re-indexes only changed files
- **Blast-radius queries:** recursive CTE finds all transitively affected files

### Repo Map (`repo_map/`)

`RepoMap::build(workspace_root)` → walks git-tracked files, extracts symbols (tree-sitter), builds reference graph.

`rank_for_agent(owned_paths, token_budget)` → `ContextPlan { full_content, signatures, repo_outline, token_estimate }`. Personalized PageRank seeds from owned paths. Outline prepended to agent prompts.

---

## 8. Write Gate

### Modes

| Mode | Behavior | Used by |
|---|---|---|
| `Interactive` | Queue proposals for TUI review (accept/reject per hunk) | Normal editor usage |
| `AutoAccept` | Accept all scope-valid writes immediately, write to disk | Swarm execution |
| `Deferred` | Accumulate proposals for batch review after agent turn | TUI agent chat |
| `RejectAll` | Silently discard all proposals | Safety fallback |

### Proposal Lifecycle

```
Created (Pending) ──► User reviews hunks ──► Accepted / PartiallyAccepted / Rejected
                                                      │
                                                      ▼
                                              Finalized: assemble final
                                              content from accepted hunks,
                                              write to disk
```

### Structural Awareness

Each hunk carries its enclosing AST node (function, class, struct, etc.), enabling `accept_node(proposal_id, "parse_config")` — accept all hunks within a named symbol.

---

## 9. ACP Integration

### Subprocess Model

Claude Code is spawned as a child process per agent session:

```
claude --print --output-format stream-json \
       --model <model> \
       --append-system-prompt <system> \
       --allowedTools Read,Glob,Grep \
       --add-dir <cwd>
```

### NDJSON Protocol (`acp/protocol.rs`)

| Type | Content |
|---|---|
| `SystemInit` | `{ session_id, model }` |
| `ContentDelta` | Streaming text chunk |
| `ToolUseStart` | `{ tool_name, tool_use_id }` |
| `AssistantMessage` | Complete message `{ text, has_tool_use }` |
| `ResultEvent` | Final result `{ is_error, result_text, duration_ms, cost_usd }` |

### AcpSessionFactory (`acp/factory.rs`)

Manages session lifecycle: `one_shot()` for single prompts, `persistent()` for multi-turn conversations, `kill_all()` for cleanup. `SessionMode` distinguishes `OneShot` from `PersistentSession`.

### Tool Restriction

Agents receive only read-only tools (`Read`, `Glob`, `Grep`). No `Edit`, `Write`, or `Bash`. All mutations flow through the Write Gate.

---

## 10. Swarm Orchestration

### Scope Validation (`swarm/validation.rs`)

`validate_scopes()` performs O(n^2) pairwise comparison of `owned_paths` across all WorkUnits. Two agents cannot own the same file or overlapping directory prefixes.

### Dependency Tiers

`dependency_tiers()` applies Kahn's algorithm to the dependency graph. Returns `Vec<Vec<String>>` — each inner vec is a tier of units that can execute in parallel.

### Tier Router (`swarm/router.rs`)

`TierRouter` maps `(ModelTier, PrivacyLevel)` → `ResolvedBackend` using `TierConfig`:
- `Cheap + Public` → Claude Haiku
- `Expensive + Public` → Claude Sonnet/Opus
- `LocalOnly` → Ollama (if enabled) or `Blocked`

### Privacy Scanner (`swarm/privacy.rs`)

`PrivacyScanner` overrides coordinator-suggested privacy levels to `LocalOnly` when file paths match configured glob patterns. Safety net — privacy is never purely LLM-determined.

### Calibration (`swarm/calibration.rs`)

`TierStats` tracks per-tier success rates across runs. Stored to memory after each swarm run for future coordinator tier-assignment improvement.

### Replanner (`swarm/replanner.rs`)

`Replanner` handles dynamic replanning after agent failures. `ReplanDecision`: `Continue`, `RetryFailed`, `RevisePlan(CompiledPlan)`, `Abort`.

### Verification (`swarm/verify/`)

Post-merge verification with four strategies:

| Strategy | Description |
|---|---|
| `StructuralOnly` | Tree-sitter parse check for ERROR/MISSING nodes |
| `DiffReview` | LLM reviews diffs (batched per-unit, per-tier, or aggregate) |
| `TestSuite` | Run test command, optionally targeted to affected files |
| `Combined` | All three in sequence with early termination + escalation |

Reports: `StructuralReport`, `DiffReviewReport`, `TestReport`, `CombinedReport`.

### Coordinator (`swarm/coordinator.rs`)

Opus-powered task decomposition. Produces `TaskDAG { units, dependency_graph, verification_strategy }`. The coordinator queries memory for prior run calibration and repo context.

### Shared Board (`swarm/board.rs`)

Agents post tagged findings during execution. Detected by parsing `[discovery: <tag>] <content>` patterns. Downstream agents receive filtered board content.

### Agent Bus (`swarm/bus.rs`)

`AgentBus` provides inter-agent communication:
- `broadcast(from, content)` → all agents via `broadcast::channel`
- `send_to(from, to, content)` → targeted via per-agent `mpsc::UnboundedSender`

### Checkpoint/Resume (`swarm/execution_state.rs`)

`ExecutionState` tracks per-node `NodeStatus` (Pending → Blocked → Ready → Running → Completed | SoftFailure | HardFailure). Serialized to `.gaviero/state/{plan_hash}.json` after each node. `--resume` loads checkpoint and skips completed nodes.

---

## 11. Tree-Sitter Pipeline

### Language Registry

16 languages: Rust, Java, JavaScript, TypeScript, HTML, CSS, JSON, Bash, TOML, C, C++, LaTeX, Python, YAML, Kotlin, Gaviero DSL. A single `LANGUAGE_REGISTRY` table is the source of truth.

### Structural Enrichment

`enrich_hunks(hunks, original, language) → Vec<StructuralHunk>`: parse original, walk AST to find enclosing named node per hunk (function, class, struct), extract identifier name, generate description.

### Syntax Highlighting (TUI)

`highlight.rs` runs tree-sitter highlight queries against the buffer's cached AST. Only the visible viewport range is processed. Queries from `queries/{lang}/highlights.scm` (Helix-sourced, MIT).

### Indentation Engine

`indent/` module: tree-sitter queries → hybrid heuristic → bracket counting fallback.

---

## 12. Terminal Subsystem

```
┌─────────────────────────────────────────┐
│            TerminalManager              │
│  ┌──────────────┐  ┌──────────────┐    │
│  │TerminalInst 0│  │TerminalInst 1│ …  │
│  │  ┌─────────┐ │  │  ┌─────────┐ │    │
│  │  │   PTY   │ │  │  │   PTY   │ │    │
│  │  │ (child) │ │  │  │ (child) │ │    │
│  │  └─────────┘ │  │  └─────────┘ │    │
│  └──────────────┘  └──────────────┘    │
│                                         │
│  event_tx ──► mpsc::Receiver ──► TUI EventLoop bridge
└─────────────────────────────────────────┘
```

Manager → Instance → PTY (`portable-pty`). vt100 for escape sequence parsing. OSC 133 for prompt/command boundary detection. Per-instance `HISTFILE`, environment isolation.

---

## 13. TUI Layout

```
┌──────────────────────────────────────────────────┐
│                    Tab Bar                        │
├────────┬──────────────────────────┬───────────────┤
│        │                          │               │
│  Left  │        Editor            │  Side Panel   │
│ Panel  │     (center, largest)    │ (Agent Chat / │
│        │                          │  Swarm Dash / │
│        │                          │  Git Panel)   │
│        │                          │               │
├────────┴──────────────────────────┴───────────────┤
│                    Terminal                        │
├───────────────────────────────────────────────────┤
│                   Status Bar                      │
└───────────────────────────────────────────────────┘
```

### Focus Model

`Focus` enum: `Editor | FileTree | SidePanel | Terminal`. Alt+Number switches focus. Ctrl = editor/text, Alt = workspace, Shift = selection extension.

### Left Panel Modes

`FileTree` (default), `Search`, `Changes`, `Review`. Review entered programmatically on agent proposals.

### Side Panel Modes

`AgentChat` (default), `SwarmDashboard`, `GitPanel`. Alt+A/W/G respectively.

### Shared Widgets

- **`ScrollState`:** scroll offset + selection with viewport caching — used by file_tree, search, swarm_dashboard
- **`TextInput`:** char-indexed text buffer with selection, undo/redo, word movement — used by agent_chat, git_panel, search, find bar

---

## 14. Concurrency Model

### Runtime

Single shared tokio runtime. All async work runs on this runtime.

### Lock Discipline

| Rule | Rationale |
|---|---|
| Never hold `WriteGatePipeline` Mutex across I/O or parsing | Prevents pipeline stalls |
| Never hold `MemoryStore` Mutex across embedding computation | ONNX inference is CPU-heavy |
| Embedding computed before lock acquisition in all store/search paths | Minimizes contention |
| Channels (mpsc, broadcast) for cross-task communication | Lock-free on the critical path |

### Shared State Pattern

```rust
Arc<tokio::sync::Mutex<T>>  // WriteGatePipeline, MemoryStore (SQLite)
Arc<dyn Observer>            // observer trait objects across tasks
mpsc::unbounded_channel      // event routing (single consumer: main loop)
broadcast::channel           // agent bus (multi-consumer)
Semaphore                    // parallel agent count bound
```

---

## 15. Configuration

### Settings Cascade

Resolution order (first match wins):
1. `{folder}/.gaviero/settings.json` — per-folder (cached)
2. `.gaviero-workspace` → `"settings"` — workspace-level
3. `~/.config/gaviero/settings.json` — user-level (cached)
4. Hardcoded Rust defaults

### Session Persistence

`SessionState` (open tabs, cursor positions, panel visibility, conversation history) persists to platform data directory, keyed by FNV-1a hash of workspace path.

---

## 16. Error Handling Strategy

- **`anyhow::Result`** for all fallible operations throughout the codebase
- Custom error types only for structured validation data: `DslError` (lexer/parser/compiler), `ScopeError`, `CycleError`
- Memory initialization failure is non-fatal: `Option<Arc<MemoryStore>>` everywhere
- Validation gate failures feed back as corrective prompts (not panics)
- Agent subprocess crashes → `AgentStatus::Failed(reason)` in manifest
- Checkpoint saves after each node completion for crash recovery
- Consolidation failures are best-effort: logged and continued

---

## 17. Hard Constraints

These are architectural invariants. Do not violate them.

1. **Write Gate mandatory** — All agent file writes pass through `WriteGatePipeline`. No direct `fs::write` from agent code paths.
2. **git2 only** — Git operations use the `git2` crate. Never shell out to `git`.
3. **Tree-sitter for everything** — Structural analysis AND syntax highlighting. No regex-based highlighter.
4. **Mutex-wrapped SQLite** — `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`. All DB methods are async.
5. **Core/TUI separation** — Pipeline logic in core. Rendering + input in TUI. Test core without TUI.
6. **Single event channel** — TUI receives all external events through one `mpsc::UnboundedReceiver<Event>`. No direct state mutation from background tasks.
7. **AutoAccept in swarm** — `WriteMode::AutoAccept` during swarm execution. User reviews aggregate result post-merge.
8. **Observer-only coupling to UI** — Core never imports TUI/CLI types. Events flow out via trait objects.
9. **Provider-agnostic backend** — `AgentBackend` + `UnifiedStreamEvent` decouple orchestration from Claude/Ollama specifics.
10. **No plugins** — Features compiled in. Configuration via settings files only.
11. **Embedding outside lock** — CPU-heavy ONNX inference always completes before acquiring the SQLite Mutex.
12. **Scope-aware memory writes** — Every `store_scoped()` call requires an explicit `WriteScope`. The system never guesses scope level.
