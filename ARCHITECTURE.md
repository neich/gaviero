# Gaviero — Architecture

Terminal editor + headless CLI for AI agent orchestration. Rust 2024.

**Binaries:** `gaviero` (TUI), `gaviero-cli` (headless)  
**Build:** `cargo build` (workspace root)

---

## 1. Topology

```
        ┌──────────────────────────────┐
        │          Workspace           │
        └──────────────────────────────┘
              │
   ┌──────────┼──────────┬─────────────┐
   ▼          ▼          ▼             ▼
┌──────┐ ┌────────┐ ┌────────┐ ┌──────────────┐
│ core │ │  tui   │ │  cli   │ │     dsl      │
│ lib  │ │ binary │ │ binary │ │     lib      │
└──┬───┘ └───┬────┘ └───┬────┘ └──────┬───────┘
   │         │          │              │
   │         └────┬─────┴──────────────┘
   │              │ (tui+cli depend on core+dsl, dsl on core)
   │              │
   │     ┌────────▼──────────────┐
   └────▶│ tree-sitter-gaviero   │  (re-exported via gaviero-core::lib)
         └───────────────────────┘
```

**Dependency rules:**
- `gaviero-core`: no UI, no DSL.
- `gaviero-dsl`: depends on `gaviero-core`.
- `gaviero-tui`, `gaviero-cli`: depend on `gaviero-core` + `gaviero-dsl`.
- `tree-sitter-gaviero`: syntax only; re-exported from `gaviero-core::lib.rs` (`Language`, `Parser`, `Tree`, `Node`, `Query`, `QueryCursor`, `Point`, `InputEdit`). Never import `tree-sitter` downstream.

| Crate | Type | Role | Key deps |
|---|---|---|---|
| `gaviero-core` | lib, 21 modules | Swarm, memory, ACP, write gate, validation, git, terminal, repo-map, context planning | tokio, tree-sitter 0.25, git2, rusqlite+sqlite-vec, ort, petgraph, ropey, portable-pty |
| `gaviero-tui` | bin (`gaviero`) | Terminal editor, panels, event routing, observers | ratatui 0.30, crossterm 0.29, notify, tui-term, core, dsl |
| `gaviero-cli` | bin (`gaviero-cli`) | Argument parsing, observer wiring, delegation | clap, tokio, core, dsl |
| `gaviero-dsl` | lib, 5 modules | `.gaviero` compiler (lexer → parser → AST → `CompiledPlan`) | logos, chumsky, miette, core |
| `tree-sitter-gaviero` | grammar | Syntax tree and highlights for `.gaviero` | tree-sitter, cc |

---

## 2. Module Map

### `gaviero-core/src/` — 21 public modules

```
lib.rs                Re-exports tree-sitter types + 21 pub mod
types.rs              FileScope, WriteProposal, ModelTier, PrivacyLevel,
                      DiffHunk, StructuralHunk, normalize_path
workspace.rs          Workspace, settings cascade, namespace resolution
session_state.rs      SessionState, TabState, PanelState, StoredConversation,
                      load_session / save_session / conversation index
tree_sitter.rs        LANGUAGE_REGISTRY (16 langs), enrich_hunks, language detection
diff_engine.rs        compute_hunks (similar crate)
write_gate.rs         WriteGatePipeline, WriteMode, proposal lifecycle
observer.rs           WriteGateObserver, AcpObserver, SwarmObserver traits
scope_enforcer.rs     FileScope checks (delegates glob matching to path_pattern)
path_pattern.rs       Glob path matching + patterns_overlap; backs DSL
                      scope-overlap validation and swarm::validation
git.rs                GitRepo, WorktreeManager, GitCoordinator (git2 only)
query_loader.rs       Tree-sitter .scm query discovery
indent/               compute_indent (tree-sitter + heuristic + bracket fallback)
terminal/             PTY lifecycle (portable-pty) + OSC 133 parsing
repo_map/             File ranking + personalized PageRank
acp/                  Claude subprocess: session / protocol / client / factory
agent_session/        Provider-session trait + per-provider impls
                      (claude, codex_exec, codex_app_server, ollama, registry)
                      — V9 refactor: transport layer consuming PlannerSelections
context_planner/      Bootstrap / delta / replay policy. Owns PlannerInput →
                      PlannerSelections (memory, graph, replay, continuity)
                      with SessionLedger and compaction
memory/               Scoped embeddings + ONNX + consolidation + code_graph
swarm/                6-phase orchestration (see §5)
iteration/            IterationEngine, retry + best-of-N + TDD
validation_gate/      Tree-sitter + cargo check gates
```

See `crates/gaviero-core/ARCHITECTURE.md` for per-subsystem detail.

### Other crates

- `gaviero-dsl/src/`: `lib.rs` / `lexer.rs` / `ast.rs` / `parser.rs` / `compiler.rs` / `error.rs`. See `crates/gaviero-dsl/ARCHITECTURE.md`.
- `gaviero-tui/src/`: `app/`, `editor/`, `panels/`, `widgets/`. See `crates/gaviero-tui/ARCHITECTURE.md`.
- `gaviero-cli/src/main.rs`: single file, ~1 KLOC. See `crates/gaviero-cli/ARCHITECTURE.md`.
- `tree-sitter-gaviero/`: `grammar.js`, generated `parser.c` / `grammar.json` / `node-types.json`, `src/lib.rs` (LANGUAGE export + tests).

---

## 3. Core Abstractions

### `FileScope` (`types.rs`)

```rust
pub struct FileScope {
    pub owned_paths: Vec<String>,
    pub read_only_paths: Vec<String>,
    pub interface_contracts: HashMap<String, String>,
}
```

Paths are glob-matched by `path_pattern::matches`. Pairwise overlap detection uses `path_pattern::patterns_overlap`, so `plans/claude-*.md` and `plans/codex-*.md` are accepted as non-overlapping while `src/` vs `src/foo.rs` is flagged.

### `WorkUnit` (`swarm/models.rs`)

Complete specification of a single agent task — scope, client/tier selection, memory routing (`read_namespaces`, `write_namespace`, `memory_importance`, `staleness_sources`, `memory_read_query`, `memory_read_limit`, `memory_write_content`), context expansion (`impact_scope`, `context_callers_of`, `context_tests_for`, `context_depth`), retry / escalation.

### `CompiledPlan` (`swarm/plan.rs`)

Immutable DAG (`petgraph::DiGraph<PlanNode, DependencyEdge>`) plus `max_parallel`, `iteration_config`, `verification_config`, `loop_configs`, `source_file`. Key methods: `work_units_ordered()` (Kahn topo-sort), `from_work_units(units)`, `hash()` (stable checkpoint id). Produced by `gaviero_dsl::compile[_with_vars]` and `swarm::coordinator`; consumed by `swarm::pipeline::execute`.

### `UnifiedStreamEvent` (`swarm/backend/mod.rs`)

Provider-agnostic event: `TextDelta`, `ThinkingDelta`, `ToolCallStart/Delta/End`, `FileBlock { path, content }`, `Usage`, `Error`, `Done`. Emitted by every `AgentBackend`; consumed by `backend::executor::{complete_to_text, complete_to_write_gate}`.

### `AgentBackend` trait

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync + 'static {
    async fn stream_completion(&self, req: CompletionRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;
    fn capabilities(&self) -> Capabilities;
    fn name(&self) -> &str;
    async fn health_check(&self) -> Result<()>;
}
```

Implementations: `ClaudeCodeBackend`, `OllamaStreamBackend`, `CodexBackend`, `MockBackend`. Model spec parsing and routing live in `swarm/backend/shared.rs` and `swarm/router.rs`.

### `PlannerSelections` / `ContextPlanner` (`context_planner/`)

Single owner of bootstrap / delta / replay policy. Returns structured `MemorySelection`, `GraphSelection`, `ReplayPayload`, `ContinuityHandle`, `ProviderProfile`, consumed by `agent_session::build_turn` to produce a `Turn` for the transport layer. Final prompt formatting happens at the provider edge.

### `MemoryScope` / `WriteScope` (`memory/scope.rs`)

5-level hierarchy — `Global → Workspace → Repo → Module → Run`. `search_scoped` cascades narrowest→widest with early exit at 0.70 confidence. `store_scoped` requires an explicit `WriteScope` — never inferred.

### Observer traits (`observer.rs`)

- `WriteGateObserver`: `on_proposal_created`, `on_proposal_updated`, `on_proposal_finalized`.
- `AcpObserver`: `on_stream_chunk`, `on_tool_call_started`, `on_streaming_status`, `on_message_complete`, `on_proposal_deferred`, `on_validation_result`, `on_validation_retry`.
- `SwarmObserver`: phase, agent state, tier, dispatch, merge conflict, coordination, cost, completion.

TUI and CLI implement all three; core never imports TUI/CLI types.

---

## 4. Data Flow — Agent Write Proposal

```
Agent stream (ACP / Ollama / Codex)
    │
    │ file block or write tool call
    ▼
AcpPipeline::propose_write(path, content)
    │
    ├─ BRIEF LOCK  write_gate: is_scope_allowed(agent_id, path)
    │              (path_pattern::matches against FileScope)
    │              release
    │
    ├─ NO LOCK     read original, diff_engine::compute_hunks
    │
    ├─ NO LOCK     tree_sitter::enrich_hunks → Vec<StructuralHunk>
    │
    ├─ BRIEF LOCK  write_gate::insert_proposal
    │              Interactive→queue+observer, AutoAccept→finalize,
    │              Deferred→batch, RejectAll→drop
    │              release
    │
    └─ NO LOCK     [finalized] fs::write
```

Lock discipline: the `WriteGatePipeline` Mutex is held only for O(1) scope checks / map inserts. All tree-sitter parsing, diff computation, and disk I/O run outside the lock.

---

## 5. Data Flow — Swarm Execution

`swarm::pipeline::execute(plan, config, memory, observer, …)` drives six phases:

```
1. VALIDATE     validate_scopes (pairwise path_pattern::patterns_overlap),
                Kahn topo-sort, dependency_tiers → Vec<Vec<WorkUnit>>
2. EXECUTE      per tier (sequential), per unit (parallel bounded by Semaphore):
                  git worktree → IterationEngine::run
                    build prompt (memory + repo_map + board + feedback)
                    backend.stream_completion → UnifiedStreamEvent
                    each FileBlock → write_gate.insert_proposal
                    ValidationPipeline (tree-sitter + cargo check [+ tests])
                    corrective retry on failure
                  checkpoint ExecutionState
3. MERGE        git merge --no-ff main; on conflict MergeResolver asks Claude
4. VERIFY       VerificationStrategy: Structural | DiffReview | TestSuite |
                Combined; escalate on failure
5. CLEANUP      WorktreeManager::teardown_all, drop gaviero/* branches
6. CONSOLIDATE  Consolidator: triage (≥0.4) → decay (30-day half-life) →
                cross-scope promotion (3+ module hits → repo, ×1.2 boost)
```

---

## 6. Concurrency Model

Single shared `tokio` runtime.

| Subsystem | Primitive | Rule |
|---|---|---|
| Write gate | `Arc<tokio::sync::Mutex<WriteGatePipeline>>` | Never hold across I/O, diff, or tree-sitter parse |
| Memory store | `Arc<tokio::sync::Mutex<rusqlite::Connection>>` | Embed outside lock; brief DB ops only |
| Execution state | `Arc<Mutex<ExecutionState>>` | Checkpoint after each node (resume-safe) |
| Agent bus | `tokio::sync::broadcast` | Lock-free multi-consumer |
| TUI events | `mpsc::unbounded_channel<Event>` | Single receiver, main loop only mutates state |
| Parallel fan-out | `Semaphore` | Bounded agent concurrency per tier |

Golden rule: **no Mutex held across `await`, tree-sitter parse, or `fs` I/O**. Embeddings (`ort`) run outside SQLite Mutex.

---

## 7. Error Handling

| Error | Crate | Strategy |
|---|---|---|
| `anyhow::Error` | all | context + propagate |
| `DslError` / `DslErrors` | dsl | `miette::Report` with source spans |
| scope violation | write_gate | reject proposal, observer callback, no retry |
| validation gate failure | validation_gate | corrective prompt → retry same agent |
| agent failure | swarm | `AgentStatus::Failed(reason)` → escalate if configured |
| merge conflict | swarm::merge | Claude resolution or user choice |
| memory init failure | memory | `Option<Arc<MemoryStore>>`, continue without memory |
| consolidation / worktree cleanup | memory, git | best-effort, log only |

---

## 8. Hard Constraints

1. All agent writes through `WriteGatePipeline`. No direct `fs::write` from agent paths.
2. `git2` only — never shell out to `git`.
3. Tree-sitter for syntax, highlighting, indent (with hybrid + bracket fallbacks). 16-language registry.
4. `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`; embedding runs outside lock.
5. Core has no UI / CLI types — coupling via observer traits only.
6. Single TUI event channel; no background task mutates `App`.
7. `AutoAccept` during swarm; user reviews post-swarm.
8. Provider-agnostic backends (`AgentBackend` + `UnifiedStreamEvent`); resolution via `TierRouter` + `PrivacyScanner`.
9. No plugin system — configuration via `settings.json` only.
10. Swarm branches: `gaviero/{work_unit_id}`. Worktrees: `.gaviero/worktrees/{id}/`, cleaned up via `Drop`.
11. Memory writes require explicit `WriteScope`; never inferred.

---

## 9. Public API Summary

```rust
// gaviero-core
pub mod acp;                 // AcpPipeline, AcpSessionFactory
pub mod agent_session;       // AgentSession, Turn, build_turn
pub mod context_planner;     // ContextPlanner, PlannerInput → PlannerSelections
pub mod memory;              // MemoryStore, MemoryScope, WriteScope, SearchConfig
pub mod swarm;               // pipeline::execute, coordinator::plan_coordinated
pub mod repo_map;            // RepoMap, build, rank_for_agent
pub mod write_gate;          // WriteGatePipeline, WriteMode
pub mod validation_gate;     // ValidationPipeline, ValidationGate
pub mod workspace;           // Workspace::open
pub mod git;                 // GitRepo, WorktreeManager, GitCoordinator
pub mod iteration;           // IterationEngine
pub mod session_state;       // SessionState, load/save/index
pub mod path_pattern;        // matches, patterns_overlap
pub mod scope_enforcer;      // FileScope permission checks
pub mod observer;            // trait definitions
pub mod types;               // domain types
pub mod tree_sitter;         // LANGUAGE_REGISTRY, enrich_hunks
pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};

// gaviero-dsl
pub fn compile(source, filename, workflow, runtime_prompt) -> Result<CompiledPlan, miette::Report>;
pub fn compile_with_vars(source, filename, workflow, runtime_prompt,
                         override_vars: &[(String, String)])
    -> Result<CompiledPlan, miette::Report>;
```

Binaries (`gaviero-tui`, `gaviero-cli`) expose no library API.

---

See [CLAUDE.md](CLAUDE.md) for conventions, build, and rules.
