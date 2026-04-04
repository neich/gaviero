# gaviero-core — Architecture

The foundational library. All domain logic lives here. `gaviero-cli`, `gaviero-tui`, and `gaviero-dsl` are thin layers that depend on this crate; none of them contain execution, validation, or memory logic.

---

## Module map

```
gaviero-core/src/
├── lib.rs                 re-exports tree-sitter types; declares all pub modules
├── types.rs               primitive domain types (see Key types below)
├── observer.rs            event-listener traits (WriteGateObserver, AcpObserver, SwarmObserver)
│
├── swarm/                 orchestration layer
│   ├── models.rs          WorkUnit, AgentManifest, SwarmResult, AgentStatus
│   ├── plan.rs            CompiledPlan, PlanNode, DependencyEdge, VerificationConfig
│   ├── pipeline.rs        execute(), plan_coordinated() — top-level entry points
│   ├── coordinator.rs     Coordinator, TaskDAG — Opus-powered task decomposition
│   ├── router.rs          TierRouter — maps (tier, privacy) → backend name
│   ├── board.rs           SharedBoard — inter-agent discovery postings
│   ├── bus.rs             AgentBus — inter-tier message passing
│   ├── validation.rs      validate_scopes(), dependency_tiers()
│   ├── execution_state.rs ExecutionState — checkpoint/resume (disk-backed JSON)
│   ├── merge.rs           merge_branch(), auto_resolve_conflicts()
│   ├── calibration.rs     TierStats, cost modelling
│   └── backend/
│       ├── mod.rs         AgentBackend trait, UnifiedStreamEvent, CompletionRequest
│       ├── runner.rs      run_backend() — inner execution + retry loop
│       ├── claude_code.rs ClaudeCodeBackend (subprocess via ACP)
│       ├── ollama.rs      OllamaStreamBackend (HTTP)
│       └── mock.rs        MockBackend (deterministic test fixture)
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
├── write_gate/
│   └── mod.rs             WriteGatePipeline, WriteMode (Interactive/AutoAccept/Deferred/RejectAll)
│
├── acp/                   ACP subprocess communication with Claude CLI
│   ├── session.rs         AcpSession — spawns `claude`, reads NDJSON stream
│   └── protocol.rs        StreamEvent (raw ACP) → UnifiedStreamEvent mapping
│
├── memory/                semantic memory store
│   ├── mod.rs             init(), MemoryStore public API
│   ├── store.rs           SQLite + sqlite-vec; store/search/mark_stale
│   ├── embedder.rs        Embedder trait
│   ├── onnx_embedder.rs   OnnxEmbedder (nomic-embed-text-v1.5, local ONNX)
│   └── schema.rs          DB schema migrations
│
├── repo_map/              PageRank-based context budget planner
│   ├── mod.rs             RepoMap::build(), rank_for_agent() → ContextPlan
│   ├── builder.rs         walks workspace, extracts file nodes + tree-sitter symbols
│   └── page_rank.rs       personalized PageRank over reference graph
│
├── scope_enforcer.rs      path-level read/write permission checks + hardcoded block-list
├── git.rs                 GitRepo, WorktreeManager, GitCoordinator (serialises .git/ ops)
├── tree_sitter.rs         15-language registry, enrich_hunks(), find_enclosing_node()
├── diff_engine.rs         compute_hunks() — LCS-based diff → Vec<DiffHunk>
├── workspace.rs           Workspace, settings load/save, namespace resolution
├── session_state.rs       persisted editor state (tabs, cursor, layout)
├── terminal/              PTY lifecycle (portable-pty + vt100)
├── query_loader.rs        tree-sitter query file loader
└── indent.rs              indentation utilities
```

---

## Key types

### Execution units

| Type | Module | Description |
|---|---|---|
| `WorkUnit` | `swarm/models.rs` | One agent task: scope, model, privacy, memory routing, retry budget, instructions. |
| `AgentManifest` | `swarm/models.rs` | Result of one agent run: status, modified files, branch name, cost_usd. |
| `SwarmResult` | `swarm/models.rs` | Aggregate: all manifests + merge results + pre-run git SHA. |
| `CompiledPlan` | `swarm/plan.rs` | Immutable petgraph DAG of `PlanNode` + `IterationConfig` + `VerificationConfig`. |
| `IterationResult` | `iteration/mod.rs` | Output of `IterationEngine::run()`: best manifest, attempt count, pass flag. |

### Configuration

| Type | Module | Description |
|---|---|---|
| `ModelTier` | `types.rs` | `Cheap` (Haiku/local) \| `Expensive` (Sonnet/Opus). |
| `PrivacyLevel` | `types.rs` | `Public` (any API) \| `LocalOnly` (Ollama only). |
| `IterationConfig` | `iteration/mod.rs` | Strategy, retry budget, cheap/expensive model names, escalation threshold. |
| `VerificationConfig` | `swarm/plan.rs` | Boolean flags: compile, clippy, test. |
| `Strategy` | `iteration/mod.rs` | `SinglePass` \| `Refine` \| `BestOfN { n }`. |
| `SwarmConfig` | `swarm/pipeline.rs` | Runtime options: workspace root, model, max parallel, namespace lists. |

### File handling

| Type | Module | Description |
|---|---|---|
| `FileScope` | `types.rs` | `owned_paths` (rw) and `read_only_paths` per agent. `to_prompt_clause()` serialises for prompts. |
| `WriteProposal` | `types.rs` | One proposed file change: original, proposed content, structural hunks, acceptance status. |
| `StructuralHunk` | `types.rs` | `DiffHunk` + enclosing tree-sitter node (function/class context). |
| `DiffHunk` | `types.rs` | Line ranges + text for one diff region. HunkType: Added/Removed/Modified. |

### Observer traits (`observer.rs`)

| Trait | Methods |
|---|---|
| `WriteGateObserver` | `on_proposal_created`, `on_proposal_updated`, `on_proposal_finalized` |
| `AcpObserver` | `on_stream_chunk`, `on_tool_call_started`, `on_streaming_status`, `on_message_complete`, `on_proposal_deferred`, `on_validation_result`, `on_validation_retry` |
| `SwarmObserver` | `on_phase_changed`, `on_agent_state_changed`, `on_tier_started`, `on_merge_conflict`, `on_completed`, `on_coordination_started`, `on_coordination_complete`, `on_tier_dispatch`, `on_cost_update` |

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
                       │     shared_board discoveries (from earlier tiers)
                       │     corrective feedback (on retry N)
                       │
                       ├─ backend.stream_completion(request)
                       │     → Stream<UnifiedStreamEvent>
                       │       TextDelta | ToolCallStart/End | FileBlock | Usage | Done
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

After all tiers: if `use_worktrees`, merge agent branches into main. Conflicts auto-resolved via Claude.

---

## Subsystem details

### AgentBackend (`swarm/backend/mod.rs`)

```rust
trait AgentBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn stream_completion(&self, req: CompletionRequest)
        -> Result<Pin<Box<dyn Stream<Item = UnifiedStreamEvent>>>>;
}
```

`UnifiedStreamEvent` variants: `TextDelta(String)`, `ThinkingDelta(String)`, `ToolCallStart { id, name }`, `ToolCallDelta { id, args_chunk }`, `ToolCallEnd { id }`, `FileBlock { path, content }`, `Usage(TokenUsage)`, `Error(String)`, `Done(StopReason)`.

Implementations: `ClaudeCodeBackend` (ACP subprocess), `OllamaStreamBackend` (HTTP), `MockBackend` (test fixture with scripted event list).

### ValidationPipeline (`validation_gate/mod.rs`)

```rust
enum ValidationResult { Pass, Fail { message: String, suggestion: Option<String> }, Skip }
```

`ValidationPipeline::default_for_rust()` = TreeSitterGate + CargoCheckGate.  
`ValidationPipeline::fast_only()` = TreeSitterGate only (used in test generation).  
Gates run sequentially; first `Fail` stops the pipeline and is fed back to the agent as a corrective prompt.

### WriteGatePipeline (`write_gate/mod.rs`)

Modes: `AutoAccept` (write immediately) | `Interactive` (per-hunk review) | `Deferred` (batch accumulate) | `RejectAll`.

Scope enforcement: every write path checked against `agent_scopes` registry (populated via `register_agent_scope`). Writes outside `owned_paths` or into the hardcoded block-list (`.env`, `.ssh/`, `id_rsa`, `.aws/`, `credentials`, …) are rejected.

### IterationEngine (`iteration/mod.rs`)

Strategy controls the outer loop:
- `SinglePass` — 1 attempt, inner `max_retries = 1`
- `Refine` — 1 attempt, inner `max_retries` from config (default 5)
- `BestOfN { n }` — up to n attempts; returns first success or the one with the most modified files; `ConvergenceDetector` stops early after 2 non-improving consecutive attempts

Model escalation: attempts `< escalate_after` → `cheap_model`; attempts `≥ escalate_after` → `expensive_model`.

### MemoryStore (`memory/mod.rs`)

SQLite + `sqlite-vec`. Per-entry metadata: namespace, privacy level, importance (0.0–1.0), source file path + hash (for staleness).

Key operations:
- `store_with_options(ns, key, content, opts)` — embed + insert
- `search_context_filtered(namespaces, query, limit, privacy)` → ranked results
- `check_staleness(paths)` — compares stored source hash vs current file hash
- `mark_stale(ids)` — sets importance=0.0 (invisible to search)

Embedder: `nomic-embed-text-v1.5` via ONNX Runtime, runs locally with no API call.

### RepoMap (`repo_map/mod.rs`)

`RepoMap::build(workspace_root)` → walks git-tracked files, extracts symbols (tree-sitter), builds reference graph.

`rank_for_agent(owned_paths, token_budget)` → `ContextPlan { repo_outline, full_content, signatures, token_estimate }`.  
`repo_outline` (ranked file summaries) is prepended to agent prompts to provide codebase context within token budget.

### SharedBoard (`swarm/board.rs`)

Agents post tagged findings during execution. Detected by parsing `[discovery: <tag>] <content>` patterns in agent text output. Downstream agents (later dependency tiers) receive filtered board content in their prompt.

### AcpSession (`acp/session.rs`)

Spawns `claude` CLI as a subprocess. Writes a JSON request on stdin; reads NDJSON `StreamEvent` lines on stdout. Translates to `UnifiedStreamEvent` for the backend abstraction layer.

---

## Dependency graph (modules)

```
pipeline.rs
  → models.rs, plan.rs, coordinator.rs
  → iteration/mod.rs → backend/runner.rs ← write_gate, validation_gate, repo_map, memory, board
  → merge.rs → acp/session.rs, git.rs
  → execution_state.rs, git.rs (WorktreeManager), memory/store.rs

write_gate → types.rs, diff_engine.rs, tree_sitter.rs, observer.rs
validation_gate → tree_sitter.rs, types.rs
repo_map → tree_sitter.rs, query_loader.rs
memory → onnx_embedder.rs (ONNX), store.rs (SQLite)
scope_enforcer → types.rs
acp/session.rs → (external: `claude` CLI binary)
git.rs → (external: git2 crate + `git` CLI)
types.rs, observer.rs → no internal deps
```

---

## Design invariants

1. **Provider-agnostic backend.** `AgentBackend` + `UnifiedStreamEvent` decouple orchestration from Claude/Ollama specifics.
2. **Observer-only coupling to UI.** Core never imports TUI/CLI types. Events flow out via trait objects.
3. **Single-agent fast path.** If `work_units.len() == 1`, `execute()` bypasses worktrees, bus, and merge, going directly to `IterationEngine`.
4. **Fail-safe memory.** Memory init failure is non-fatal; all call sites accept `Option<&MemoryStore>`.
5. **Scope + privacy layering.** `ScopeEnforcer` enforces file-path rules; `PrivacyScanner` enforces model-selection rules.
6. **Checkpoint/resume.** `ExecutionState` serialises to `.gaviero/state/<plan-hash>.json` after every node.
7. **Async throughout.** Tokio runtime; no blocking calls on async threads; `GitCoordinator` serialises `git` CLI ops to avoid `.git/index.lock` races.
