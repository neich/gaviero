# gaviero-core — Architecture

The foundational library. All domain logic lives here. `gaviero-cli`, `gaviero-tui`, and `gaviero-dsl` are thin layers that depend on this crate; none of them contain execution, validation, or memory logic.

---

## Module map

```
gaviero-core/src/
├── lib.rs                 re-exports tree-sitter types; declares 18 pub modules
├── types.rs               primitive domain types (see Key types below)
├── observer.rs            event-listener traits (WriteGateObserver, AcpObserver, SwarmObserver)
│
├── swarm/                 orchestration layer
│   ├── mod.rs             module declarations (18 sub-modules)
│   ├── models.rs          WorkUnit, AgentManifest, AgentStatus, SwarmResult, MergeResult
│   ├── plan.rs            CompiledPlan, PlanNode, DependencyEdge, TriggerRule, VerificationConfig,
│   │                      LoopConfig, LoopUntilCondition
│   ├── pipeline.rs        execute(), plan_coordinated() — top-level entry points, SwarmConfig
│   ├── coordinator.rs     Coordinator, CoordinatorConfig, TaskDAG — Opus-powered task decomposition
│   ├── planner.rs         plan_task() — natural language → WorkUnit decomposition (legacy)
│   ├── router.rs          TierRouter, TierConfig, LocalConfig, ResolvedBackend
│   ├── privacy.rs         PrivacyScanner — glob-based privacy enforcement
│   ├── board.rs           SharedBoard — inter-agent discovery postings
│   ├── bus.rs             AgentBus — inter-tier message passing (broadcast + targeted)
│   ├── context.rs         ContextConfig, RepoContext, build_context() — memory-aware context assembly
│   ├── validation.rs      validate_scopes(), dependency_tiers() (Kahn's algorithm)
│   ├── execution_state.rs ExecutionState, NodeStatus — checkpoint/resume (disk-backed JSON)
│   ├── merge.rs           merge_branch(), auto_resolve_conflicts()
│   ├── calibration.rs     TierStats — per-tier success rate tracking
│   ├── replanner.rs       Replanner, ReplanDecision — dynamic replanning after failures
│   ├── ollama.rs          Ollama-specific configuration
│   ├── backend/
│   │   ├── mod.rs         AgentBackend trait, UnifiedStreamEvent, CompletionRequest, Capabilities
│   │   ├── runner.rs      run_backend() — inner execution + retry loop
│   │   ├── claude_code.rs ClaudeCodeBackend (subprocess via ACP)
│   │   ├── ollama.rs      OllamaStreamBackend (HTTP SSE)
│   │   └── mock.rs        MockBackend (deterministic test fixture)
│   └── verify/
│       ├── mod.rs         VerificationStrategy, BatchStrategy, CombinedReport, CostEstimate,
│       │                  StructuralReport, DiffReviewReport, TestReport, EscalationRecord
│       ├── structural.rs  Tree-sitter parse validation (ERROR/MISSING node detection)
│       ├── diff_review.rs LLM-based diff review (Sonnet reviews agent diffs)
│       ├── test_runner.rs Test suite execution + output parsing
│       └── combined.rs    Combined strategy: structural → diff review → test suite
│
├── iteration/             outer execution strategy loop
│   ├── mod.rs             IterationEngine, IterationConfig, Strategy, IterationResult
│   ├── convergence.rs     ConvergenceDetector — stall detection across attempts
│   └── test_generator.rs  TestGenerator — TDD red-phase test file generation
│
├── validation_gate/       post-edit correctness gates
│   ├── mod.rs             ValidationPipeline, ValidationGate trait, ValidationResult
│   ├── tree_sitter_gate.rs fast syntax check (per-write)
│   └── cargo_gate.rs      slow semantic check (cargo check, per-checkpoint)
│
├── write_gate.rs          WriteGatePipeline, WriteMode (Interactive/AutoAccept/Deferred/RejectAll)
│
├── acp/                   ACP subprocess communication
│   ├── mod.rs             module declarations
│   ├── session.rs         AcpSession — spawns `claude`, reads NDJSON stream; AgentOptions
│   ├── protocol.rs        StreamEvent (raw ACP) → UnifiedStreamEvent mapping; NDJSON parsing
│   ├── client.rs          AcpPipeline — prompt enrichment, file block detection, proposal routing
│   └── factory.rs         AcpSessionFactory — session lifecycle (one_shot, persistent, kill_all)
│
├── memory/                semantic memory store
│   ├── mod.rs             init(), MemoryStore public API
│   ├── store.rs           SQLite + sqlite-vec; store/search/mark_stale, PrivacyFilter, StoreOptions
│   ├── embedder.rs        Embedder trait
│   ├── onnx_embedder.rs   OnnxEmbedder (ONNX Runtime, mean pooling, L2 normalization)
│   ├── model_manager.rs   ONNX model download + caching (~/.cache/gaviero/models/)
│   ├── schema.rs          DB schema migrations
│   ├── code_graph.rs      CodeGraph: petgraph-based symbol graph, SQLite persistence
│   └── consolidation.rs   Consolidator: dedup (>0.85), merge flagging (0.7-0.85), pruning
│
├── repo_map/              PageRank-based context budget planner + code knowledge graph
│   ├── mod.rs             RepoMap::build(), rank_for_agent() → ContextPlan; FileNode, Symbol
│   ├── builder.rs         walks workspace, extracts file nodes + tree-sitter symbols
│   ├── page_rank.rs       personalized PageRank over reference graph
│   ├── edges.rs           extract_rust_references(): use/call/impl/test edges from AST
│   ├── graph_builder.rs   incremental knowledge graph builder (hash-based change detection)
│   └── store.rs           GraphStore: SQLite-backed node/edge storage, blast-radius queries (CTE)
│
├── scope_enforcer.rs      path-level read/write permission checks + hardcoded block-list
├── git.rs                 GitRepo, WorktreeManager, GitCoordinator (serialises .git/ ops)
├── tree_sitter.rs         16-language registry, enrich_hunks(), find_enclosing_node()
├── diff_engine.rs         compute_hunks() — similar crate wrapper → Vec<DiffHunk>
├── workspace.rs           Workspace, WorkspaceFolder, settings cascade (cached)
├── session_state.rs       persisted editor state (tabs, cursor, layout)
├── terminal/              PTY lifecycle (portable-pty + vt100)
│   ├── mod.rs             exports, Manager → Instance hierarchy
│   ├── types.rs           TerminalId, ShellState, CommandRecord
│   ├── config.rs          ShellConfig, ShellType, TerminalConfig
│   ├── instance.rs        TerminalInstance: individual PTY tab
│   ├── manager.rs         TerminalManager: lifecycle, multi-instance coordination
│   ├── pty.rs             Pseudo-terminal allocation and I/O
│   ├── session.rs         Terminal session state persistence
│   ├── event.rs           TerminalEvent types
│   ├── osc.rs             OSC 133 sequence parsing (prompt/command detection)
│   ├── context.rs         Terminal context (cwd, env)
│   ├── history.rs         Command history tracking
│   └── shell_integration.rs Shell integration protocol
├── query_loader.rs        tree-sitter query file loader
└── indent/                indentation utilities
    ├── mod.rs             compute_indent() entry point, IndentResult
    ├── treesitter.rs      Tree-sitter-based indent
    ├── heuristic.rs       Hybrid indent (relative delta)
    ├── bracket.rs         Bracket-counting fallback
    ├── captures.rs        Tree-sitter capture processing
    ├── predicates.rs      Indent rule predicates
    ├── config.rs          Indent configuration
    └── utils.rs           Shared indent utilities
```

---

## Key types

### Execution units

| Type | Module | Description |
|---|---|---|
| `WorkUnit` | `swarm/models.rs` | One agent task: scope, model, tier, privacy, memory routing, retry budget, graph context, instructions. |
| `AgentManifest` | `swarm/models.rs` | Result of one agent run: status, modified files, branch name, cost_usd, output text. |
| `SwarmResult` | `swarm/models.rs` | Aggregate: all manifests + merge results + pre-swarm SHA + success flag. |
| `CompiledPlan` | `swarm/plan.rs` | Immutable petgraph DAG of `PlanNode` + `IterationConfig` + `VerificationConfig` + `LoopConfig`s. |
| `IterationResult` | `iteration/mod.rs` | Output of `IterationEngine::run()`: best manifest, attempt count, pass flag. |
| `TaskDAG` | `swarm/coordinator.rs` | Coordinator output: units, dependency_graph, verification_strategy. |

### Configuration

| Type | Module | Description |
|---|---|---|
| `ModelTier` | `types.rs` | `Cheap` (Haiku/local) \| `Expensive` (Sonnet/Opus). Default: `Cheap`. |
| `PrivacyLevel` | `types.rs` | `Public` (any API) \| `LocalOnly` (Ollama only). Default: `Public`. |
| `IterationConfig` | `iteration/mod.rs` | Strategy, max_retries, max_attempts, test_first, escalate_after, cheap/expensive model names. |
| `VerificationConfig` | `swarm/plan.rs` | Boolean flags: compile, clippy, test, impact_tests. |
| `Strategy` | `iteration/mod.rs` | `SinglePass` \| `Refine` (default) \| `BestOfN { n }`. |
| `SwarmConfig` | `swarm/pipeline.rs` | Runtime: workspace root, model, max_parallel, namespaces, context_files. |
| `TierConfig` | `swarm/router.rs` | Per-tier model names, max_parallel, local backend config. |

### File handling

| Type | Module | Description |
|---|---|---|
| `FileScope` | `types.rs` | `owned_paths` (rw), `read_only_paths`, `interface_contracts`. `to_prompt_clause()` for prompts. |
| `WriteProposal` | `types.rs` | Proposed file change: original/proposed content, structural hunks, status. |
| `StructuralHunk` | `types.rs` | `DiffHunk` + enclosing tree-sitter node (function/class) + status. |
| `DiffHunk` | `types.rs` | Line ranges + text for one diff region. `HunkType`: Added/Removed/Modified. |

### Observer traits (`observer.rs`)

| Trait | Key Methods |
|---|---|
| `WriteGateObserver` | `on_proposal_created`, `on_proposal_updated`, `on_proposal_finalized` |
| `AcpObserver` | `on_stream_chunk`, `on_tool_call_started`, `on_streaming_status`, `on_message_complete`, `on_proposal_deferred`, `on_permission_request`, `on_validation_result`, `on_validation_retry` |
| `SwarmObserver` | `on_phase_changed`, `on_agent_state_changed`, `on_tier_started`, `on_merge_conflict`, `on_completed`, `on_coordination_started`, `on_coordination_complete`, `on_tier_dispatch`, `on_escalation`, `on_verification_*`, `on_cost_update` |

### Verification types (`swarm/verify/`)

| Type | Description |
|---|---|
| `VerificationStrategy` | `StructuralOnly` \| `DiffReview { review_tiers, batch_strategy }` \| `TestSuite { command, targeted }` \| `Combined` (default) |
| `BatchStrategy` | `PerUnit` \| `PerDependencyTier` (default) \| `Aggregate` |
| `CombinedReport` | Structural + DiffReview + TestSuite results + escalations + cost |
| `EscalationRecord` | unit_id, reason, from_tier → to_tier, succeeded |
| `CostEstimate` | Token counts per category + estimated USD |

---

## Execution flow

```
CompiledPlan
    │
    └─► swarm::pipeline::execute()
            │
            ├─ validate_scopes()              overlap detection
            │
            ├─ [1 unit] ─► single-agent fast path ─────────────────────────┐
            │               skip worktrees, bus, merge; go straight to      │
            │               IterationEngine                                 │
            │                                                               │
            ├─ [N units] dependency_tiers()   topological sort              │
            │   FOR EACH TIER (parallel, semaphore-bounded):                │
            │     FOR EACH WorkUnit → run_single_agent()                    │
            │                                                               ◄┘
            ▼
    IterationEngine::run()                  outer strategy loop
        │
        ├─ [test_first] TestGenerator::generate()
        │     run_backend() with TDD prompt → writes test files
        │
        └─ FOR attempt in 0..n_attempts:
               escalate model if attempt ≥ escalate_after
               │
               └─► run_backend()            inner execution + retry loop
                       │
                       ├─ build_prompt()
                       │     memory context (semantic search across namespaces)
                       │     file scope clause
                       │     repo_map outline (PageRank-ranked file summaries)
                       │     graph context (callers_of, tests_for queries)
                       │     shared_board discoveries (from earlier tiers)
                       │     corrective feedback (on retry N)
                       │
                       ├─ backend.stream_completion(request)
                       │     → Stream<UnifiedStreamEvent>
                       │       TextDelta | ThinkingDelta | ToolCall* | FileBlock | Usage | Done
                       │
                       ├─ FOR EACH FileBlock:
                       │     write_gate.insert_proposal()
                       │       AutoAccept → write immediately
                       │       Interactive/Deferred → queue for review
                       │
                       └─ ValidationPipeline::run(modified_files)
                             TreeSitterGate (fast, always)
                             CargoCheckGate (slow, optional)
                             PASS → done
                             FAIL → corrective_prompt → retry (up to max_retries)
```

After all tiers: if `use_worktrees`, merge agent branches into main. Conflicts auto-resolved via Claude. Then optional Phase 4 verification per `VerificationStrategy`.

---

## Subsystem details

### AgentBackend (`swarm/backend/mod.rs`)

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

`UnifiedStreamEvent` variants: `TextDelta(String)`, `ThinkingDelta(String)`, `ToolCallStart { id, name }`, `ToolCallDelta { id, args_chunk }`, `ToolCallEnd { id }`, `FileBlock { path, content }`, `Usage(TokenUsage)`, `Error(String)`, `Done(StopReason)`.

`Capabilities` flags: `tool_use`, `streaming`, `vision`, `extended_thinking`, `max_context_tokens`, `supports_system_prompt`, `supports_file_blocks`.

Implementations: `ClaudeCodeBackend` (ACP subprocess), `OllamaStreamBackend` (HTTP SSE), `MockBackend` (scripted events for tests).

### ValidationPipeline (`validation_gate/mod.rs`)

```rust
enum ValidationResult { Pass, Fail { message: String, suggestion: Option<String> }, Skip }
```

`ValidationPipeline::default_for_rust()` = TreeSitterGate + CargoCheckGate.
`ValidationPipeline::fast_only()` = TreeSitterGate only (used in test generation).
Gates run sequentially; first `Fail` stops the pipeline and is fed back as a corrective prompt.

### WriteGatePipeline (`write_gate.rs`)

Modes: `AutoAccept` (write immediately) | `Interactive` (per-hunk review) | `Deferred` (batch accumulate) | `RejectAll`.

Scope enforcement: every write path checked against `agent_scopes` registry. Writes outside `owned_paths` or into the hardcoded block-list (`.env`, `.ssh/`, `id_rsa`, `.aws/`, `credentials`, ...) are rejected.

### IterationEngine (`iteration/mod.rs`)

Strategy controls the outer loop:
- `SinglePass` — 1 attempt, inner `max_retries = 1`
- `Refine` (default) — 1 attempt, inner `max_retries` from config (default 5)
- `BestOfN { n }` — up to n attempts; returns first success or best by modified file count; `ConvergenceDetector` stops early after 2 non-improving consecutive attempts

Model escalation: attempts `< escalate_after` → `cheap_model`; attempts `≥ escalate_after` → `expensive_model`.

### TierRouter (`swarm/router.rs`)

Maps `(ModelTier, PrivacyLevel)` → `ResolvedBackend`:
- `Cheap + Public` → `Claude { model: cheap_model }` (default: "haiku")
- `Expensive + Public` → `Claude { model: expensive_model }` (default: "sonnet")
- `* + LocalOnly` → `Ollama { model, base_url }` if enabled, else `Blocked`
- Explicit `WorkUnit.model` → bypass router entirely

### PrivacyScanner (`swarm/privacy.rs`)

Glob-based safety net. Patterns like `**/clinical/**`, `.env*` force `PrivacyLevel::LocalOnly` on matching file paths. Overrides coordinator suggestions — privacy is never purely LLM-determined.

### MemoryStore (`memory/`)

SQLite + `sqlite-vec`. Per-entry metadata: namespace, privacy level, importance (0.0–1.0), source file path + hash (staleness).

Key operations:
- `store_with_options(ns, key, content, opts)` — embed + insert
- `search_context_filtered(namespaces, query, limit, privacy)` → ranked results
- `check_staleness(paths)` — compares stored source hash vs current file hash
- `mark_stale(ids)` — sets importance=0.0 (invisible to search)

Consolidation on store: similarity >0.85 reinforces existing entry, 0.7–0.85 flags for LLM merge, <0.7 normal insert.

Embedder: ONNX Runtime (`ort`) + `tokenizers`, runs locally with no API call. L2-normalized output vectors.

### Code Knowledge Graph (`repo_map/store.rs`, `repo_map/edges.rs`, `repo_map/graph_builder.rs`)

SQLite-backed directed graph:
- **Node kinds:** `File`, `Function`, `Struct`, `Trait`, `Enum`, `Test`
- **Edge kinds:** `Imports`, `Calls`, `Implements`, `TestedBy`, `Contains`
- **Incremental:** `graph_builder.rs` compares SHA-256 file hashes, re-indexes only changed files
- **Reference extraction:** `edges.rs` uses tree-sitter AST walking to find `use`/`import`, function calls, `impl Trait for Type`, test annotations
- **Blast radius:** recursive CTE queries find all transitively affected files from a set of changed paths
- **Impact scope:** `WorkUnit.impact_scope = true` auto-expands `read_only_paths` with blast-radius files

### RepoMap (`repo_map/mod.rs`)

`RepoMap::build(workspace_root)` → walks git-tracked files, extracts symbols (tree-sitter), builds reference graph.

`rank_for_agent(owned_paths, token_budget)` → `ContextPlan { repo_outline, full_content, signatures, token_estimate }`.
Personalized PageRank seeds from owned paths. `repo_outline` prepended to agent prompts within token budget.

### SharedBoard (`swarm/board.rs`)

Agents post tagged findings during execution. Detected by parsing `[discovery: <tag>] <content>` patterns in agent text output. Downstream agents (later dependency tiers) receive filtered board content.

### AcpSession (`acp/session.rs`)

Spawns `claude` CLI as a subprocess. Writes JSON request on stdin; reads NDJSON `StreamEvent` lines on stdout. `AgentOptions` configures model, system prompt, allowed tools, working directory.

### AcpSessionFactory (`acp/factory.rs`)

Manages session lifecycle:
- `one_shot(prompt, options)` — single prompt, subprocess exits after response
- `persistent(id)` → `PersistentSession` — multi-turn, subprocess stays alive
- `send_to_persistent(id, prompt)` — send to existing session
- `kill_session(id)` / `kill_all()` — cleanup

### ExecutionState (`swarm/execution_state.rs`)

Per-node lifecycle: `Pending → Blocked → Ready → Running → Completed | SoftFailure | HardFailure`.

Serialized to `.gaviero/state/{plan_hash}.json` after each node. On `--resume`, completed nodes are skipped.

### Replanner (`swarm/replanner.rs`)

After exhausting retries on a `HardFailure`, asks Opus to revise the remaining plan. `ReplanDecision`: `Continue` | `RetryFailed(ids)` | `RevisePlan(CompiledPlan)` | `Abort(reason)`.

### Coordinator Context (`swarm/context.rs`)

`build_context(workspace_root, prompt, memory)` assembles repo context for the coordinator:
- File tree listing (capped at `max_files`)
- Memory summaries substituted for raw content where hash matches HEAD
- Key files included directly
- Token budget enforcement

---

## Dependency graph (modules)

```
pipeline.rs
  → models.rs, plan.rs, coordinator.rs
  → iteration/mod.rs → backend/runner.rs ← write_gate, validation_gate, repo_map, memory, board
  → merge.rs → acp/session.rs, git.rs
  → execution_state.rs, git.rs (WorktreeManager), memory/store.rs
  → verify/ (structural, diff_review, test_runner, combined)
  → calibration.rs, replanner.rs

coordinator.rs → planner.rs (extract_json), validation.rs, verify/mod.rs
                → acp/session.rs, memory/store.rs

router.rs → models.rs, types.rs
privacy.rs → models.rs, types.rs
context.rs → memory/store.rs

write_gate → types.rs, diff_engine.rs, tree_sitter.rs, observer.rs, scope_enforcer.rs
validation_gate → tree_sitter.rs, types.rs
repo_map → tree_sitter.rs, query_loader.rs, store.rs (SQLite), edges.rs, graph_builder.rs
memory → onnx_embedder.rs (ONNX), store.rs (SQLite), code_graph.rs, consolidation.rs
scope_enforcer → types.rs
acp/session.rs → (external: `claude` CLI binary)
acp/factory.rs → acp/session.rs
acp/client.rs → write_gate, diff_engine, tree_sitter, observer, acp/session
git.rs → (external: git2 crate)
types.rs, observer.rs → no internal deps
```

---

## Design invariants

1. **Provider-agnostic backend.** `AgentBackend` + `UnifiedStreamEvent` decouple orchestration from Claude/Ollama specifics.
2. **Observer-only coupling to UI.** Core never imports TUI/CLI types. Events flow out via trait objects.
3. **Single-agent fast path.** If `work_units.len() == 1`, `execute()` bypasses worktrees, bus, and merge, going directly to `IterationEngine`.
4. **Fail-safe memory.** Memory init failure is non-fatal; all call sites accept `Option<&MemoryStore>` or `Option<Arc<MemoryStore>>`.
5. **Scope + privacy layering.** `ScopeEnforcer` enforces file-path rules; `PrivacyScanner` enforces model-selection rules. Both are safety nets over LLM decisions.
6. **Checkpoint/resume.** `ExecutionState` serialises to `.gaviero/state/<plan-hash>.json` after every node.
7. **Async throughout.** Tokio runtime; no blocking calls on async threads; `GitCoordinator` serialises `git` CLI ops to avoid `.git/index.lock` races.
8. **Incremental graph.** Code knowledge graph re-indexes only changed files (SHA-256 hash comparison).
