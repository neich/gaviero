# Gaviero — Architecture

Terminal editor for AI agent orchestration. Rust 2024.

**Binaries:** `gaviero` (TUI), `gaviero-cli` (headless swarm runner)
**Platform:** Linux (primary), POSIX with modern terminal
**Build:** `cargo build` from workspace root

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

Pipeline logic -> `gaviero-core`. TUI = render + input. CLI = args + observers. DSL = `.gaviero` compilation. Core testable without UI.

| Crate | Role | Key deps |
|---|---|---|
| `gaviero-core` | Write gate, diff, tree-sitter, ACP, swarm, memory, git, terminal, repo map, iteration, validation | tokio, tree-sitter 0.25 + 16 langs, git2, rusqlite + sqlite-vec, ort, petgraph, similar, ropey, portable-pty, vt100, reqwest, async-trait, futures, sha2 |
| `gaviero-tui` | TUI editor binary | gaviero-core, gaviero-dsl, ratatui 0.30, crossterm 0.29, notify, arboard, tui-term |
| `gaviero-cli` | Headless swarm runner | gaviero-core, gaviero-dsl, clap, tokio |
| `gaviero-dsl` | `.gaviero` DSL compiler | gaviero-core, logos, chumsky, miette, thiserror |
| `tree-sitter-gaviero` | Tree-sitter grammar for `.gaviero` highlighting | tree-sitter (build-time C grammar) |

Tree-sitter types re-exported from `gaviero-core::lib.rs`. Downstream never depends `tree-sitter` directly.

---

## 2. Module Map

### gaviero-core/src/ (18 public modules)

```
lib.rs                      Re-exports tree-sitter types; 18 pub modules
types.rs                    FileScope, DiffHunk, HunkType, WriteProposal, StructuralHunk, NodeInfo,
                            SymbolKind, ModelTier, PrivacyLevel, TierAnnotation, EntryMetadata
workspace.rs                Workspace model, WorkspaceFolder, settings cascade (cached)
session_state.rs            SessionState, TabState, PanelState, StoredConversation
tree_sitter.rs              LANGUAGE_REGISTRY (16 langs), enrich_hunks(), language_for/name_for_extension()
diff_engine.rs              compute_hunks() — similar crate -> Vec<DiffHunk>
write_gate.rs               WriteGatePipeline, WriteMode, proposal management
observer.rs                 WriteGateObserver, AcpObserver, SwarmObserver trait defs
scope_enforcer.rs           Path-level read/write permission checks + hardcoded block-list
git.rs                      GitRepo (git2 wrapper), WorktreeManager, GitCoordinator, FileStatus
query_loader.rs             Tree-sitter .scm file discovery (env var -> exe dir -> cwd -> bundled)
indent/                     compute_indent() — tree-sitter, hybrid heuristic, bracket fallback
terminal/                   TerminalManager -> TerminalInstance -> PTY (portable-pty + vt100)
acp/                        AcpSession, AcpPipeline, AcpSessionFactory, NDJSON protocol
memory/                     MemoryStore (SQLite + sqlite-vec), OnnxEmbedder, CodeGraph, Consolidator,
                            MemoryScope (5-level), SearchConfig, ScoredMemory
repo_map/                   RepoMap, FileNode, ContextPlan — PageRank context + GraphStore
iteration/                  IterationEngine, Strategy, IterationConfig, ConvergenceDetector, TestGenerator
validation_gate/            ValidationPipeline, ValidationGate trait, TreeSitterGate, CargoCheckGate
swarm/                      Pipeline, Coordinator, TierRouter, AgentBus, SharedBoard, verify/, backend/
```

### gaviero-tui/src/

```
main.rs                     Entry point, terminal setup, event loop, panic handler
app.rs                      App state, layout, focus management, find bar (~5000 lines)
event.rs                    Event enum (43+ variants), EventLoop (crossterm/watcher/tick/terminal)
keymap.rs                   KeyEvent -> Action mapping; chord-prefix support
theme.rs                    ~80 colour constants (One Dark), timing constants

editor/
  buffer.rs                 Ropey buffer, Cursor, Transaction, undo/redo, find_next/prev_match
  view.rs                   EditorView: gutter, syntax highlights, cursor, scrollbar
  diff_overlay.rs           DiffSource, DiffReviewState, accept/reject per hunk
  highlight.rs              Tree-sitter highlight query runner -> Vec<StyledSpan>
  markdown.rs               Markdown rendering and editing

panels/
  file_tree.rs              Multi-root file browser, git + proposal decorations
  agent_chat.rs             AgentChatState, Conversation, attachments, @file autocomplete
  chat_markdown.rs          ChatLine: markdown rendering for chat
  swarm_dashboard.rs        Agent status table with tier/phase labels
  git_panel.rs              GitPanelState, staging, commit, branch picker
  terminal.rs               Terminal rendering (tui-term), TerminalSelectionState
  status_bar.rs             Mode, file, branch, agent status indicators
  search.rs                 SearchPanelState, interactive input + live results

widgets/
  tabs.rs                   TabBar with close indicators
  scrollbar.rs              Custom scrollbar widget
  scroll_state.rs           ScrollState: shared scroll offset + selection for list panels
  text_input.rs             TextInput: text editing with cursor, selection, undo/redo
  render_utils.rs           Shared rendering utilities
```

### gaviero-cli/src/

```
main.rs                     Cli struct (clap), CliAcpObserver, CliSwarmObserver, pipeline launcher
```

### gaviero-dsl/src/

```
lib.rs                      compile(source, filename, workflow, runtime_prompt) -> Result<CompiledPlan>
lexer.rs                    Token enum (logos), lex()
ast.rs                      Script, Item, ClientDecl, AgentDecl, WorkflowDecl, ContextBlock, ScopeBlock,
                            MemoryBlock, VerifyBlock, LoopBlock, UntilCondition, StrategyLit, TierLit
parser.rs                   parse() — chumsky combinators
compiler.rs                 compile_ast() — 7-phase semantic analysis
error.rs                    DslError (Lex/Parse/Compile), DslErrors (miette wrapper)
```

---

## 3. Core Abstractions

### FileScope (`types.rs`)

Agent permission boundary over filesystem.

```
FileScope {
    owned_paths: Vec<String>                   Writable files/dirs
    read_only_paths: Vec<String>               Read-only files/dirs
    interface_contracts: HashMap<String,String> API contracts to preserve
}
```

Used by: Write Gate (scope check), Swarm (overlap detection), Agent Runner (prompt enrichment), Memory (module path).

### WorkUnit (`swarm/models.rs`)

Single task for one swarm agent.

```
WorkUnit {
    id: String                                 Unique ID
    description: String                        Task description
    scope: FileScope                           Read/write boundaries
    depends_on: Vec<String>                    Prerequisite IDs
    backend: AgentBackend                      ClaudeCode | Ollama | Custom (deprecated)
    model: Option<String>                      Per-unit model override
    tier: ModelTier                             Cheap | Expensive
    privacy: PrivacyLevel                      Public | LocalOnly
    coordinator_instructions: String           Subtask instructions
    max_retries: u8                             Max retries before escalation
    escalation_tier: Option<ModelTier>         Tier on failure

    // Memory routing
    read_namespaces: Option<Vec<String>>       Namespaces to read
    write_namespace: Option<String>            Namespace to write
    memory_importance: Option<f32>             Importance 0.0-1.0
    staleness_sources: Vec<String>             Paths for staleness checks
    memory_read_query: Option<String>          Custom search query
    memory_read_limit: Option<usize>           Custom result limit
    memory_write_content: Option<String>       Memory write template

    // Graph / impact
    impact_scope: bool                         Auto-expand read_only via blast-radius
    context_callers_of: Vec<String>            Files for caller graph
    context_tests_for: Vec<String>             Paths for test queries
    context_depth: u32                          BFS depth (default: 2)
}
```

### CompiledPlan (`swarm/plan.rs`)

Immutable execution plan. Output of `gaviero_dsl::compile()`, input to `swarm::pipeline::execute()`.

```
CompiledPlan {
    graph: DiGraph<PlanNode, DependencyEdge>   petgraph DAG
    max_parallel: Option<usize>                 Concurrency cap
    source_file: Option<PathBuf>               Source .gaviero file
    iteration_config: IterationConfig          Strategy + retry + escalation
    verification_config: VerificationConfig    compile/clippy/test/impact flags
    loop_configs: Vec<LoopConfig>              Loop configurations
}
```

Key methods: `work_units_ordered()` (Kahn's topo sort), `from_work_units()` (flat list -> DAG), `hash()` (stable checkpoint name).

### WriteProposal (`types.rs`)

```
WriteProposal {
    id: u64                           Unique ID
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

Implementations: `ClaudeCodeBackend` (ACP subprocess), `OllamaStreamBackend` (HTTP SSE), `MockBackend` (test fixture).

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

Single `tokio::sync::mpsc::unbounded_channel<Event>` carries all external events to main loop. No background task mutates app state directly.

### Event Sources

| Source | Events | Mechanism |
|---|---|---|
| Crossterm reader | `Key`, `Mouse`, `Paste`, `Resize` | Blocking thread -> channel |
| File watcher (notify) | `FileChanged`, `FileTreeChanged` | Callback -> channel |
| Tick timer | `Tick` (~33ms, ~30fps) | tokio::interval -> channel |
| Terminal bridge | `Terminal(TerminalEvent)` | TerminalManager mpsc -> event channel |
| WriteGateObserver | `ProposalCreated/Updated/Finalized` | Observer trait impl |
| AcpObserver | `StreamChunk`, `ToolCallStarted`, `StreamingStatus`, `MessageComplete`, `FileProposalDeferred`, `AcpTaskCompleted` | Observer trait impl |
| SwarmObserver | `SwarmPhaseChanged/AgentStateChanged/TierStarted/MergeConflict/Completed/CoordinationStarted/CoordinationComplete/TierDispatch/CostUpdate/DslPlanReady` | Observer trait impl |
| Memory init | `MemoryReady` | Background spawn -> channel |

### Observer Bridge

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
    // drain up to 64 pending events before redraw
    if app.should_quit { break; }
}
```

---

## 5. Data Flow: Agent Write Proposal

Every agent write passes through this pipeline.

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
    │     ├─ Interactive -> queue, fire on_proposal_created() -> TUI shows diff
    │     ├─ AutoAccept -> accept all, return Some((path, content))
    │     ├─ Deferred -> accumulate for batch review
    │     └─ RejectAll -> discard silently
    │     Release lock
    │
    └─ 4. NO LOCK: if AutoAccept, write to disk
```

Lock discipline: WriteGatePipeline Mutex never held across I/O, tree-sitter, or diff.

---

## 6. Data Flow: Swarm Execution

```
  swarm::pipeline::execute(plan, config, checkpoint, memory, observer, make_obs)
    │
    ├─ Phase 1: VALIDATE
    │   validate_scopes() -> no owned_path overlaps (O(n^2) pairwise)
    │   work_units_ordered() -> Kahn's topo sort
    │   dependency_tiers() -> Vec<Vec<WorkUnit>> (parallel groups)
    │
    ├─ Phase 2: EXECUTE (per tier, sequential)
    │   │
    │   │  Tier N: [A, B, C] (parallel)
    │   │
    │   ├─ Per WorkUnit (bounded by Semaphore):
    │   │   ├─ Provision git worktree (branch: gaviero/{id})
    │   │   ├─ IterationEngine::run()
    │   │   │   ├─ [test_first] TestGenerator::generate()
    │   │   │   └─ FOR attempt in 0..n_attempts:
    │   │   │       escalate model if attempt >= escalate_after
    │   │   │       └─> run_backend()
    │   │   │           ├─ build_prompt()
    │   │   │           │   scoped memory (cascading search)
    │   │   │           │   file scope clause
    │   │   │           │   repo_map outline (PageRank)
    │   │   │           │   shared_board discoveries
    │   │   │           │   corrective feedback (on retry)
    │   │   │           ├─ backend.stream_completion(request)
    │   │   │           │   -> Stream<UnifiedStreamEvent>
    │   │   │           ├─ FOR EACH FileBlock: write_gate.insert_proposal()
    │   │   │           └─ ValidationPipeline::run(modified_files)
    │   │   │               TreeSitterGate -> CargoCheckGate
    │   │   │               PASS -> done | FAIL -> corrective -> retry
    │   │   └─ Broadcast to AgentBus + post to SharedBoard
    │   │
    │   └─ Checkpoint ExecutionState after each node
    │
    ├─ Phase 3: MERGE (if use_worktrees)
    │   Per successful branch: git merge --no-ff -> main
    │   On conflict: MergeResolver queries Claude
    │
    ├─ Phase 4: VERIFY (optional, per VerificationStrategy)
    │   StructuralOnly | DiffReview | TestSuite | Combined
    │   Escalation on failure -> re-run at higher tier
    │
    ├─ Phase 5: WORKTREE CLEANUP
    │   WorktreeManager::teardown_all()
    │
    ├─ Phase 6: MEMORY CONSOLIDATION (best-effort)
    │   Consolidator::consolidate_run(run_id, repo_id)
    │     Promotes durable memories to module/repo scope
    │     Deletes ephemeral run memories
    │
    └─ Return SwarmResult { manifests, merge_results, success, pre_swarm_sha }
```

### Loop Execution

Plans with `LoopConfig`: agents repeat until `LoopUntilCondition` met or `max_iterations` reached. Conditions: `Verify` (compile/test pass), `Agent` (judge returns pass), `Command` (exit 0).

---

## 7. Memory System

### Scope Hierarchy

Five levels, broadest to narrowest:

```
global (0)             Personal cross-workspace (~/.config/gaviero/memory.db)
  └─ workspace (1)     Business-level project (<workspace>/.gaviero/memory.db)
       └─ repo (2)     Single git repo
            └─ module (3)  Crate/package/subdir (FileScope.owned_paths)
                 └─ run (4)   Single swarm execution (ephemeral, consolidated up)
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
│    memories_fts (FTS5, porter+unicode61 tokenizer)              │
│    memory_access_log (cross-scope promotion heuristics)         │
│    episodes (agent run tracking)                                 │
│    graph_state (code knowledge graph serialization)             │
└──────────────────────────────────────────────────────────────────┘
```

### Key Types

| Type | Module | Description |
|---|---|---|
| `MemoryScope` | `memory/scope.rs` | Resolved scope chain: global_db, workspace_db, workspace_id, repo_id, module_path, run_id |
| `WriteScope` | `memory/scope.rs` | Write target: `Global` \| `Workspace` \| `Repo{repo_id}` \| `Module{repo_id,module_path}` \| `Run{repo_id,run_id}` |
| `ScopeFilter` | `memory/scope.rs` | Filter for single scope level in cascading search |
| `WriteMeta` | `memory/scope.rs` | memory_type, importance, trust, source, tag |
| `Trust` | `memory/scope.rs` | `High` (user, 1.2x) \| `Medium` (agent, 1.0x) \| `Low` (consolidated, 0.7x) |
| `MemoryType` | `memory/scope.rs` | `Factual` \| `Procedural` \| `Decision` \| `Pattern` \| `Gotcha` |
| `StoreResult` | `memory/scope.rs` | `Inserted(id)` \| `Deduplicated(id)` \| `AlreadyCovered` |
| `SearchConfig` | `memory/scoring.rs` | query + max_results + per_level_limit + similarity/confidence thresholds + scope chain |
| `ScoredMemory` | `memory/scoring.rs` | Memory entry with raw_similarity, fts_rank, final_score |

### Data Flow: Scoped Search

```
  Caller (build_prompt / Coordinator / MergeResolver)
    │
    │  memory.search_scoped(config: &SearchConfig)
    │
    ├─ 1. NO LOCK: embedder.embed(query_text) -> Vec<f32>   [CPU-heavy, ONNX]
    │
    ├─ 2. CASCADE through scope levels (narrowest -> widest):
    │     FOR EACH level in scope.levels() [run -> module -> repo -> workspace -> global]:
    │       │
    │       ├─ BRIEF LOCK: vec_search_at_level(embedding, level, per_level_limit)
    │       │   SELECT from vec_memories_scoped WHERE scope_level = ?
    │       │   Release lock
    │       │
    │       ├─ [if use_fts] BRIEF LOCK: fts_search_at_level(query_text, level)
    │       │   SELECT from memories_fts INNER JOIN memories
    │       │   Release lock
    │       │
    │       ├─ NO LOCK: merge_rrf(vec, fts, k=60)
    │       │   RRF: vector 70% + FTS 30%
    │       │
    │       ├─ NO LOCK: score each candidate:
    │       │   final = (similarity*0.50 + importance*0.20 + recency*0.15 + 0.15)
    │       │           * scope_weight * trust_weight * access_reinforcement
    │       │
    │       └─ EARLY TERMINATION: best_score > confidence_threshold (0.70)
    │          Stop widening — narrow results sufficient
    │
    ├─ 3. NO LOCK: deduplicate by content_hash across levels
    │
    └─ 4. Return top-K ScoredMemory, sorted by final_score desc
```

### Data Flow: Scoped Write

```
  Caller (agent observation, user /remember, consolidation)
    │
    │  memory.store_scoped(scope: &WriteScope, content, meta: &WriteMeta)
    │
    ├─ 1. NO LOCK: embedder.embed(content) -> Vec<f32>
    │
    ├─ 2. NO LOCK: content_hash = SHA-256(normalized(content))
    │
    ├─ 3. BRIEF LOCK: check dedup
    │     SELECT FROM memories WHERE content_hash = ? AND scope_level <= ?
    │     ├─ Exact match same scope -> reinforce (update importance, access_count)
    │     │   -> StoreResult::Deduplicated(id)
    │     ├─ Exact match broader scope -> skip
    │     │   -> StoreResult::AlreadyCovered
    │     └─ No match -> INSERT with scope metadata
    │        -> StoreResult::Inserted(id)
    │
    └─ 4. BRIEF LOCK: insert vec_memories_scoped + memories_fts (trigger)
```

### Consolidation Pipeline

Runs after each swarm execution (Phase 6):

```
Consolidator::consolidate_run(run_id, repo_id)
  │
  ├─ Phase 1: RUN TRIAGE
  │   query_by_run(run_id) -> Vec<ScoredMemory>
  │   FOR EACH with importance >= 0.4:
  │     store_scoped(Module or Repo, content, WriteMeta::consolidation())
  │     -> Inserted | Deduplicated | AlreadyCovered
  │   delete_by_run(run_id) — remove ephemeral
  │
  ├─ Phase 2: IMPORTANCE DECAY + PRUNING
  │   decay_and_prune()
  │   Exponential: importance *= exp(-0.023 * days_since_access)
  │   Half-life: 30 days. Prune below threshold.
  │
  └─ Phase 3: CROSS-SCOPE PROMOTION
      find_promotion_candidates(min_cross_hits=3)
      Module memories accessed by 3+ different modules
      -> promote to Repo scope, 1.2x importance boost
```

### Schema Migrations

| Ver | Changes |
|---|---|
| v1 | `memories` table, namespace/key indexes |
| v2 | `privacy` column for tier routing |
| v3 | `importance`, `access_count`, `source_file/hash`, `vec_memories` (sqlite-vec), `episodes`, `graph_state`. Nullified embeddings for model change (384d->768d). |
| v4 | Scope columns (`scope_level`, `scope_path`, `repo_id`, `module_path`, `run_id`, `content_hash`, `memory_type`, `trust`, `tag`). FTS5 + triggers. `vec_memories_scoped`. `memory_access_log`. Content hash backfill. Namespace->scope migration. |

### Embedder

- **Trait:** `embed(text) -> Vec<f32>`, `dimensions()`, `model_id()`
- **OnnxEmbedder:** `ort` (ONNX Runtime) + `tokenizers`, mean pooling, L2 norm
- **Model:** nomic-embed-text-v1.5 (768d)
- **Vector search:** sqlite-vec `vec0` virtual table, cosine distance

### Code Knowledge Graph (`memory/code_graph.rs`, `repo_map/store.rs`)

SQLite-backed directed graph:
- **Nodes:** `File`, `Function`, `Struct`, `Trait`, `Enum`, `Test` — qualified names + file hashes
- **Edges:** `Imports`, `Calls`, `Implements`, `TestedBy`, `Contains`
- **Incremental:** compare file hashes, re-index changed only
- **Blast-radius:** recursive CTE for transitively affected files

### Repo Map (`repo_map/`)

`RepoMap::build(workspace_root)` -> walks git-tracked files, extracts symbols (tree-sitter), builds reference graph.

`rank_for_agent(owned_paths, token_budget)` -> `ContextPlan { full_content, signatures, repo_outline, token_estimate }`. Personalized PageRank from owned paths. Outline prepended to agent prompts.

---

## 8. Write Gate

### Modes

| Mode | Behavior | Used by |
|---|---|---|
| `Interactive` | Queue for TUI review (accept/reject per hunk) | Editor |
| `AutoAccept` | Accept scope-valid writes immediately, write to disk | Swarm |
| `Deferred` | Accumulate for batch review after agent turn | TUI agent chat |
| `RejectAll` | Silently discard | Safety fallback |

### Proposal Lifecycle

```
Created (Pending) --> User reviews hunks --> Accepted / PartiallyAccepted / Rejected
                                                      │
                                                      ▼
                                              Finalized: assemble final
                                              content from accepted hunks,
                                              write to disk
```

### Structural Awareness

Each hunk carries enclosing AST node (function, class, struct). Enables `accept_node(proposal_id, "parse_config")` — accept all hunks within named symbol.

---

## 9. ACP Integration

### Subprocess Model

Claude Code spawned as child per agent session:

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
| `AssistantMessage` | Complete `{ text, has_tool_use }` |
| `ResultEvent` | Final `{ is_error, result_text, duration_ms, cost_usd }` |

### AcpSessionFactory (`acp/factory.rs`)

Session lifecycle: `one_shot()` for single prompts, `persistent()` for multi-turn, `kill_all()` for cleanup. `SessionMode`: `OneShot` | `PersistentSession`.

### Tool Restriction

Agents get read-only tools (`Read`, `Glob`, `Grep`). No `Edit`/`Write`/`Bash`. All mutations through Write Gate.

---

## 10. Swarm Orchestration

### Scope Validation (`swarm/validation.rs`)

`validate_scopes()`: O(n^2) pairwise comparison of `owned_paths`. Two agents cannot own same file or overlapping dir prefixes.

### Dependency Tiers

`dependency_tiers()`: Kahn's algorithm on dependency graph -> `Vec<Vec<String>>` (parallel groups per tier).

### Tier Router (`swarm/router.rs`)

`TierRouter` maps `(ModelTier, PrivacyLevel)` -> `ResolvedBackend` via `TierConfig`:
- `Cheap + Public` -> Claude Haiku
- `Expensive + Public` -> Claude Sonnet/Opus
- `LocalOnly` -> Ollama (if enabled) or `Blocked`

### Privacy Scanner (`swarm/privacy.rs`)

`PrivacyScanner` overrides privacy to `LocalOnly` when paths match configured globs. Safety net — privacy never purely LLM-determined.

### Calibration (`swarm/calibration.rs`)

`TierStats` tracks per-tier success rates across runs. Stored to memory for future tier-assignment.

### Replanner (`swarm/replanner.rs`)

`Replanner` handles dynamic replanning after failures. `ReplanDecision`: `Continue`, `RetryFailed`, `RevisePlan(CompiledPlan)`, `Abort`.

### Verification (`swarm/verify/`)

| Strategy | Description |
|---|---|
| `StructuralOnly` | Tree-sitter parse check for ERROR/MISSING nodes |
| `DiffReview` | LLM reviews diffs (batched per-unit/tier/aggregate) |
| `TestSuite` | Run test command, optionally targeted to affected files |
| `Combined` | All three sequential, early termination + escalation |

Reports: `StructuralReport`, `DiffReviewReport`, `TestReport`, `CombinedReport`.

### Coordinator (`swarm/coordinator.rs`)

Opus-powered task decomposition -> `TaskDAG { units, dependency_graph, verification_strategy }`. Queries memory for calibration + repo context.

### Shared Board (`swarm/board.rs`)

Agents post tagged findings during execution. Pattern: `[discovery: <tag>] <content>`. Downstream agents get filtered board content.

### Agent Bus (`swarm/bus.rs`)

`AgentBus` inter-agent communication:
- `broadcast(from, content)` -> all agents via `broadcast::channel`
- `send_to(from, to, content)` -> targeted via per-agent `mpsc::UnboundedSender`

### Checkpoint/Resume (`swarm/execution_state.rs`)

`ExecutionState` tracks per-node `NodeStatus` (Pending -> Blocked -> Ready -> Running -> Completed | SoftFailure | HardFailure). Serialized to `.gaviero/state/{plan_hash}.json` after each node. `--resume` loads checkpoint, skips completed.

---

## 11. Tree-Sitter Pipeline

### Language Registry

16 languages: Rust, Java, JavaScript, TypeScript, HTML, CSS, JSON, Bash, TOML, C, C++, LaTeX, Python, YAML, Kotlin, Gaviero DSL. Single `LANGUAGE_REGISTRY` table.

### Structural Enrichment

`enrich_hunks(hunks, original, language) -> Vec<StructuralHunk>`: parse original, walk AST for enclosing named node per hunk, extract identifier, generate description.

### Syntax Highlighting (TUI)

`highlight.rs` runs tree-sitter queries against buffer's cached AST. Only visible viewport processed. Queries from `queries/{lang}/highlights.scm` (Helix-sourced, MIT).

### Indentation

`indent/`: tree-sitter queries -> hybrid heuristic -> bracket counting fallback.

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
│  event_tx --> mpsc::Receiver --> TUI EventLoop bridge
└─────────────────────────────────────────┘
```

Manager -> Instance -> PTY (`portable-pty`). vt100 for escape parsing. OSC 133 for prompt/command boundaries. Per-instance `HISTFILE`, env isolation.

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

`Focus` enum: `Editor | FileTree | SidePanel | Terminal`. Alt+Number switches. Ctrl = editor/text, Alt = workspace, Shift = selection.

### Left Panel Modes

`FileTree` (default), `Search`, `Changes`, `Review`. Review entered on agent proposals.

### Side Panel Modes

`AgentChat` (default), `SwarmDashboard`, `GitPanel`. Alt+A/W/G.

### Shared Widgets

- **`ScrollState`:** scroll offset + selection with viewport caching — file_tree, search, swarm_dashboard
- **`TextInput`:** char-indexed buffer with selection, undo/redo, word movement — agent_chat, git_panel, search, find bar

---

## 14. Concurrency Model

### Runtime

Single shared tokio runtime for all async work.

### Lock Discipline

| Rule | Rationale |
|---|---|
| Never hold WriteGatePipeline Mutex across I/O or parsing | Prevents stalls |
| Never hold MemoryStore Mutex across embedding | ONNX is CPU-heavy |
| Embedding computed before lock in all store/search | Minimizes contention |
| Channels (mpsc, broadcast) for cross-task comm | Lock-free critical path |

### Shared State

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

Resolution order (first wins):
1. `{folder}/.gaviero/settings.json` — per-folder (cached)
2. `.gaviero-workspace` -> `"settings"` — workspace-level
3. `~/.config/gaviero/settings.json` — user-level (cached)
4. Hardcoded Rust defaults

### Session Persistence

`SessionState` (tabs, cursors, panels, conversations) persists to platform data dir, keyed by FNV-1a hash of workspace path.

---

## 16. Error Handling

- `anyhow::Result` for all fallible ops
- Custom errors only for structured validation: `DslError`, `ScopeError`, `CycleError`
- Memory init failure non-fatal: `Option<Arc<MemoryStore>>` everywhere
- Validation failures -> corrective prompts (not panics)
- Agent crashes -> `AgentStatus::Failed(reason)`
- Checkpoint after each node for crash recovery
- Consolidation failures best-effort: logged, continued

---

## 17. Hard Constraints

Architectural invariants. Do not violate.

1. **Write Gate mandatory** — All agent writes through `WriteGatePipeline`. No direct `fs::write` from agent paths.
2. **git2 only** — Never shell out to `git`.
3. **Tree-sitter for everything** — Structural analysis AND highlighting. No regex highlighter.
4. **Mutex-wrapped SQLite** — `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`. All DB methods async.
5. **Core/TUI separation** — Pipeline in core. Render + input in TUI. Test core without TUI.
6. **Single event channel** — TUI gets all external events through one `mpsc::UnboundedReceiver<Event>`. No direct state mutation from background.
7. **AutoAccept in swarm** — `WriteMode::AutoAccept` during swarm. User reviews aggregate post-merge.
8. **Observer-only coupling** — Core never imports TUI/CLI types. Events via trait objects.
9. **Provider-agnostic backend** — `AgentBackend` + `UnifiedStreamEvent` decouple orchestration from provider specifics.
10. **No plugins** — Features compiled in. Config via settings only.
11. **Embedding outside lock** — ONNX inference completes before acquiring SQLite Mutex.
12. **Scope-aware writes** — Every `store_scoped()` needs explicit `WriteScope`. Never guess scope.
