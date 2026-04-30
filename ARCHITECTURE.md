# Gaviero — Architecture

Terminal editor + headless CLI for AI agent orchestration. Rust 2024.

**Binaries:** `gaviero` (TUI), `gaviero-cli` (headless), `gaviero-mcp-shim` (stdio↔socket bridge)
**Build:** `cargo build` (workspace root)

---

## 1. Topology

```
                ┌──────────────────────────────┐
                │          Workspace           │
                └──────────────┬───────────────┘
                               │
   ┌───────┬─────────┬─────────┼─────────┬─────────────┐
   ▼       ▼         ▼         ▼         ▼             ▼
┌──────┐ ┌────┐ ┌────────┐ ┌──────┐ ┌──────────┐ ┌──────────────┐
│ core │ │tui │ │  cli   │ │ dsl  │ │ mcp-shim │ │ tree-sitter- │
│ lib  │ │bin │ │  bin   │ │ lib  │ │   bin    │ │   gaviero    │
└──┬───┘ └─┬──┘ └───┬────┘ └──┬───┘ └────┬─────┘ └──────┬───────┘
   │       │        │         │          │              │
   │       └───┬────┴─────────┘          │              │
   │           │ tui+cli depend on       │              │
   │           │ core+dsl; dsl on core   │              │
   │           │ shim is standalone      │              │
   │           ▼                         │              │
   │    .gaviero/mcp.sock ◀──────────────┘              │
   │    (Unix socket)    stdio bridge                   │
   │                                                    │
   └─── re-exports ──────────────────────────────────► tree-sitter types
```

**Dependency rules:**
- `gaviero-core`: no UI, no DSL deps. Hosts the in-process MCP server (`mcp::spawn_mcp_server`).
- `gaviero-dsl`: depends on `gaviero-core` for `CompiledPlan` / `path_pattern` / `WorkUnit`.
- `gaviero-tui`, `gaviero-cli`: depend on `gaviero-core` + `gaviero-dsl`.
- `gaviero-mcp-shim`: standalone — `tokio` / `clap` / `anyhow` / `tracing` only. Speaks to a running Gaviero exclusively over its Unix socket.
- `tree-sitter-gaviero`: grammar only; re-exported from `gaviero-core::lib` (`Language`, `Parser`, `Tree`, `Node`, `Query`, `QueryCursor`, `Point`, `InputEdit`). Never import `tree-sitter` downstream.

| Crate | Type | Role | Key deps |
|---|---|---|---|
| `gaviero-core` | lib (21 pub mods) | Swarm, memory, ACP, MCP server, write gate, validation, git, terminal, repo-map, context planner, agent-session transport | tokio, tree-sitter 0.25, git2 0.19, rusqlite + sqlite-vec, ort 2.0 + tokenizers, petgraph 0.8, ropey, portable-pty, rmcp |
| `gaviero-tui` | bin (`gaviero`) | Editor, panels, observer wiring, slash-commands | ratatui 0.30, crossterm 0.29, notify, tui-term, core, dsl |
| `gaviero-cli` | bin (`gaviero-cli`) | Argument parsing, observer wiring, eval / sleeptime / forget tooling | clap 4, tokio, core, dsl |
| `gaviero-dsl` | lib (6 pub mods) | `.gaviero` compiler (lexer → parser → AST → `CompiledPlan`) | logos, chumsky, miette, core |
| `gaviero-mcp-shim` | bin (`gaviero-mcp-shim`) | stdio↔Unix-socket bridge for subprocess agents (Claude Code, Codex) → core MCP server | tokio, clap |
| `tree-sitter-gaviero` | grammar | Tree-sitter grammar + queries for `.gaviero` | tree-sitter, cc |

---

## 2. Module Map (`gaviero-core/src/`, 21 pub mods)

```
lib.rs                  Re-exports tree-sitter types + 21 pub mods
types.rs                FileScope, WriteProposal, ModelTier, PrivacyLevel,
                        DiffHunk, StructuralHunk, NodeInfo, normalize_path
workspace.rs            Workspace, settings cascade, namespace resolution
session_state.rs        SessionState, TabState, PanelState, StoredConversation,
                        ConversationIndex, load/save_session
tree_sitter.rs          LANGUAGE_REGISTRY (16 langs), enrich_hunks
diff_engine.rs          compute_hunks (similar)
write_gate.rs           WriteGatePipeline, WriteMode, proposal lifecycle
observer.rs             WriteGateObserver, AcpObserver, SwarmObserver traits
scope_enforcer.rs       FileScope checks (delegates to path_pattern)
path_pattern.rs         Glob matcher + patterns_overlap (DSL + runtime)
git.rs                  GitRepo, WorktreeManager, GitCoordinator (git2)
query_loader.rs         Tree-sitter .scm query discovery
indent/                 compute_indent (tree-sitter + hybrid + bracket)
terminal/               PTY (portable-pty) + OSC 133 parsing
repo_map/               Code knowledge graph + personalized PageRank
acp/                    Claude subprocess transport (NDJSON ACP protocol)
agent_session/          V9 transport layer: AgentSession trait + per-provider
                        impls (claude, codex_exec, codex_app_server, ollama,
                        registry); LegacyAgentSession shim wraps AcpPipeline
context_planner/        Bootstrap / delta / replay policy. Owns memory, graph,
                        replay, continuity; emits PlannerSelections
mcp/                    In-process MCP server (Tier A). Three read-only tools
                        (memory_search, blast_radius, node_doc) over a Unix
                        socket; .mcp.json / .codex/config.toml synthesis;
                        external-server detection
memory/                 Multi-DB scoped memory: pluggable Embedder, single
                        writer task, RRF hybrid retrieval, three-cadence
                        consolidation, soft-delete, eval, telemetry
swarm/                  6-phase orchestration (validate → execute → merge →
                        verify → cleanup → consolidate); pluggable backends;
                        replanner, calibration, context_bundle
iteration/              IterationEngine (retry, BestOfN, TDD)
validation_gate/        ValidationPipeline (TreeSitterGate + CargoCheckGate)
```

See `crates/gaviero-core/ARCHITECTURE.md` for per-subsystem detail.

### Other crates

- `gaviero-dsl/src/`: `lib.rs` / `lexer.rs` / `ast.rs` / `parser.rs` / `compiler.rs` / `error.rs`. See `crates/gaviero-dsl/ARCHITECTURE.md`.
- `gaviero-tui/src/`: `app/`, `editor/`, `panels/` (incl. `memory_panel.rs`), `widgets/`. See `crates/gaviero-tui/ARCHITECTURE.md`.
- `gaviero-cli/src/main.rs`: single file, ~2.1 KLOC. See `crates/gaviero-cli/ARCHITECTURE.md`.
- `gaviero-mcp-shim/src/main.rs`: ~110 lines; bidirectional `tokio::io::copy` between stdio and `<workspace>/.gaviero/mcp.sock`, with reconnect/backoff up to `--connect-timeout-secs`. Pure transport.
- `tree-sitter-gaviero/`: `grammar.js` + generated `parser.c` / `grammar.json` / `node-types.json`, `src/lib.rs` (LANGUAGE export).

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

Glob-matched by `path_pattern::matches`. Pairwise overlap uses `path_pattern::patterns_overlap` — `plans/claude-*.md` vs `plans/codex-*.md` is non-overlapping; `src/` vs `src/foo.rs` is flagged.

### `WorkUnit` (`swarm/models.rs`)

Full agent task spec — scope, client/tier/privacy, retries + escalation, memory routing (`read_namespaces`, `write_namespace`, `memory_importance`, `staleness_sources`, `memory_read_query`, `memory_read_limit`, `memory_write_content`), context expansion (`impact_scope`, `context_callers_of`, `context_tests_for`, `context_depth`).

### `CompiledPlan` (`swarm/plan.rs`)

`DiGraph<PlanNode, DependencyEdge>` + `max_parallel`, `iteration_config`, `verification_config`, `loop_configs`, `source_file`. Methods: `work_units_ordered()` (Kahn topo-sort), `from_work_units()`, `hash()` (stable checkpoint id). Produced by `gaviero_dsl::compile[_with_vars]` and `swarm::coordinator`; consumed by `swarm::pipeline::execute`.

### `UnifiedStreamEvent` (`swarm/backend/mod.rs`)

Provider-agnostic event: `TextDelta | ThinkingDelta | ToolCallStart/Delta/End | FileBlock { path, content } | Usage | Error | Done`. Emitted by every `AgentBackend`; consumed by `swarm::backend::executor::{complete_to_text, complete_to_write_gate}`.

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

Implementations: `ClaudeCodeBackend`, `OllamaStreamBackend`, `CodexBackend` (dual-mode: `codex exec` and `codex app-server`), `MockBackend`. Model-spec parsing in `swarm/backend/shared.rs`; tier resolution in `swarm/router.rs`.

### `AgentSession` trait + `Turn` (`agent_session/`)

V9 transport boundary. `Turn` is a lossless lift of `PlannerSelections` plus a `TransportContext` (user message, effort, auto-approve). Implementations: `ClaudeSession`, `CodexExecSession` (StatelessReplay), `CodexAppServerSession` (ProcessBound continuity), `OllamaSession`, `LegacyAgentSession` (shim around `AcpPipeline` for byte-identical migration). Routing in `agent_session::registry::SessionConstruction`.

### `PlannerSelections` / `ContextPlanner` (`context_planner/`)

Single owner of bootstrap / delta / replay policy. Emits `MemorySelection`, `GraphSelection`, `FileAttachment`, `ReplayPayload`, `ContinuityHandle`, `ProviderProfile`. Lifted into a `Turn` by `agent_session::build_turn`. Final prompt formatting happens at the provider edge. `chat_memory.rs` wires per-turn memory injection + the post-turn extractor pipeline.

### `MemoryScope` / `WriteScope` (`memory/scope.rs`)

5-level hierarchy: `Global → Workspace → Repo → Module → Run`. Stored across three SQLite files (global at `~/.config/gaviero/memory.db`, workspace+run at `<workspace>/.gaviero/memory.db`, repo+module at `<folder>/.gaviero/memory.db`) — coordinated through `MemoryStores` (multi-DB registry). `WriteScope` is always explicit.

### `GavieroMcpServer` (`mcp/server.rs`)

In-process MCP server task. Three read-only tools — `memory_search`, `blast_radius`, `node_doc`. Listens on `<workspace>/.gaviero/mcp.sock`; subprocess agents reach it via `gaviero-mcp-shim`. Read-only by construction: there is no `WriterHandle` on the server type, so `memory_store` / `_update` / `_delete` are unimplementable. Per-worktree `.mcp.json` (Claude) and `.codex/config.toml` (Codex, behind `TrustConsent`) are synthesized by `mcp::config_synth`. `mcp::external_memory` detects and optionally disables competing memory MCP servers in the agent config.

### Observer traits (`observer.rs` + `memory/observer.rs` + `mcp/observer.rs`)

- `WriteGateObserver`: `on_proposal_created / updated / finalized`.
- `AcpObserver`: stream chunks, tool-call lifecycle, validation results, retries, deferred proposals.
- `SwarmObserver`: phase, agent state, tier, dispatch, merge conflict, coordination, cost, completion.
- `MemoryObserver`: write committed, deletion, restore.
- `ManifestObserver`: per-turn `injection_manifests` row persisted (drives the TUI memory panel "Injected Now" section).
- `McpToolCallObserver`: tool-call audit log.

TUI and CLI implement all relevant traits; core never imports TUI/CLI types.

---

## 4. Data Flow — Agent Write Proposal

```
Agent stream (ACP / Codex / Ollama)
    │
    │ FileBlock event or write tool call
    ▼
AgentSession::send_turn → UnifiedStreamEvent → executor::complete_to_write_gate
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

The `WriteGatePipeline` Mutex is held only for O(1) scope checks / map inserts. Tree-sitter parsing, diff, and disk I/O run outside the lock.

---

## 5. Data Flow — Swarm Execution

`swarm::pipeline::execute(plan, config, memory, observer, …)` drives six phases:

```
1. VALIDATE      validate_scopes (pairwise path_pattern::patterns_overlap),
                 Kahn topo-sort, dependency_tiers → Vec<Vec<WorkUnit>>
                 SwarmContextBundle built once: 1 shared memory query +
                 per-unit graph slices (avoids N+1 queries).
2. EXECUTE       per tier (sequential), per unit (parallel via Semaphore):
                   git worktree (gaviero/{unit_id}) →
                   per-worktree MCP config synth (.mcp.json / .codex/) →
                   IterationEngine::run:
                     ContextPlanner::plan → PlannerSelections → Turn
                     AgentSession::send_turn → UnifiedStreamEvent stream
                     each FileBlock → WriteGatePipeline::insert_proposal
                     ValidationPipeline (tree-sitter + cargo check [+ tests])
                     corrective retry on failure
                   checkpoint ExecutionState (resume-safe)
3. MERGE         git merge --no-ff main; on conflict MergeResolver asks Claude
4. VERIFY        Structural | DiffReview | TestSuite | Combined; escalate on fail
5. CLEANUP       WorktreeManager::teardown_all, drop gaviero/* branches,
                 remove per-worktree MCP configs
6. CONSOLIDATE   per-run consolidation (triage ≥ 0.4 → decay → cross-scope
                 promotion); calibration::TierStats persisted to memory
```

`Replanner::evaluate` is a Phase-3 stub: today `ReplanDecision::Continue` is always returned; the wired-up version will compile a coordinator-generated `.gaviero` revision into a fresh `CompiledPlan`.

---

## 6. Data Flow — Memory (write + retrieve)

```
WRITE  caller → WriterHandle::send(WriterMessage)
                       │ (mpsc, bounded)
                       ▼
       Single writer task (memory/writer.rs)
                       │
                       ├─ embed (outside any lock)
                       ├─ BRIEF LOCK   dedup probe + insert
                       │               (vec_memories_scoped + memories_fts)
                       └─ optional oneshot ack (500ms timeout)

RETRIEVE  caller → memory::retrieve_ranked / retrieve_for_chat
                       │
                       ├─ embed query (outside lock)
                       ├─ MemoryStores::search_scoped
                       │     mode = merged   (B3 default): RRF over all admitted
                       │                                   scopes; vec 0.7 + fts 0.3
                       │     mode = cascade  (kill-switch): narrow→wide, exit at 0.70
                       ├─ optional reranker (sigmoid-calibrated logits blended w/
                       │                     composite); off by default
                       └─ persist `injection_manifests` row (S4) → ManifestObserver
```

Three-cadence consolidation: per-turn `extractor.rs` (Tier S3) → per-session `session_consolidator.rs` (B5) → idle/weekly `sleeptime.rs` + `sleeptime_scheduler.rs` (decay sweep, near-dup merge, cross-scope promotion, trust re-scoring, history compression, summary prune).

Soft-delete via `/forget` writes to a `deletions` audit table; History rows are immutable except via the C2.4 redaction path. Restores replay through the dedup pipeline.

---

## 7. Concurrency Model

Single shared `tokio` runtime.

| Subsystem | Primitive | Rule |
|---|---|---|
| Write gate | `Arc<tokio::sync::Mutex<WriteGatePipeline>>` | No lock across I/O, diff, parse |
| Memory store(s) | `tokio::sync::Mutex<rusqlite::Connection>` per DB | Embed outside lock; brief DB ops only; all writes funnel through the writer task |
| MCP graph cache | `tokio::sync::Mutex<GraphStore>` | Lazy first build, reused across `blast_radius` calls |
| Execution state | `Arc<Mutex<ExecutionState>>` | Checkpoint after each node (resume-safe) |
| Agent bus | `tokio::sync::broadcast` | Lock-free multi-consumer |
| TUI events | `mpsc::unbounded_channel<Event>` | Single receiver mutates `App` |
| Parallel fan-out | `Semaphore` | Bounded agent concurrency per tier |

Golden rule: **no Mutex held across `await`, tree-sitter parse, or `fs` I/O**. ONNX inference (`ort`) runs outside SQLite Mutex. `#![deny(clippy::await_holding_lock)]` is enforced in `memory/writer.rs`.

---

## 8. Error Handling

| Error | Crate | Strategy |
|---|---|---|
| `anyhow::Error` | all | context + propagate |
| `DslError` / `DslErrors` | dsl | `miette::Report` with source spans |
| scope violation | write_gate | reject proposal, observer callback, no retry |
| validation gate failure | validation_gate | corrective prompt → retry same agent |
| agent failure | swarm | `AgentStatus::Failed(reason)` → escalate or replan |
| merge conflict | swarm::merge | Claude resolution or user choice |
| memory init failure | memory | `Option<Arc<MemoryStores>>`, continue without memory |
| C1 schema migration | memory | refuse open without explicit consent (`--accept-c1-migration` / TUI prompt) |
| MCP server bind failure | mcp | log; subprocess agents fall back to prompt-time injection |
| consolidation / worktree cleanup | memory, git | best-effort, log only |

---

## 9. Hard Constraints

1. All agent writes flow through `WriteGatePipeline`. No direct `fs::write` from agent paths.
2. `git2` only — never shell out to `git`.
3. Tree-sitter for syntax, highlighting, indent. 16-language registry.
4. `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`; embedding runs outside the lock; all writes go through the single writer task (`WriterMessage` mpsc).
5. MCP is read-only by construction — no write tools, ever (Phase 1 invariant).
6. Core has no UI / CLI types — coupling via observer traits only.
7. Single TUI event channel; no background task mutates `App`.
8. `AutoAccept` during swarm; user reviews post-swarm.
9. Provider-agnostic backends + transport (`AgentBackend` + `UnifiedStreamEvent`, `AgentSession` + `Turn`); resolution via `TierRouter` + `PrivacyScanner`.
10. Swarm branches: `gaviero/{work_unit_id}`. Worktrees: `.gaviero/worktrees/{id}/`, cleaned up via `Drop`.
11. Memory writes require explicit `WriteScope`; never inferred.
12. `WorkUnit` scope overlap is checked against `path_pattern::patterns_overlap` so glob-disjoint siblings are accepted.

---

## 10. Public API Summary

```rust
// gaviero-core (21 pub mods)
pub mod acp;                 // AcpPipeline, AcpSessionFactory
pub mod agent_session;       // AgentSession, Turn, build_turn, registry
pub mod context_planner;     // ContextPlanner, PlannerInput → PlannerSelections,
                             // chat_memory (per-turn injection + extractor)
pub mod mcp;                 // GavieroMcpServer, spawn_mcp_server,
                             // McpConfigSynth, ExternalMemoryServer, tool I/O types
pub mod memory;              // MemoryStores, WriterHandle, retrieve_ranked,
                             // SleeptimeScheduler, eval, MemoryKind, …
pub mod swarm;               // pipeline::execute, coordinator::plan_coordinated,
                             // SwarmContextBundle, TierStats, Replanner
pub mod repo_map;            // RepoMap, BlastRadiusMode, ImpactSummary, EdgeWeights
pub mod write_gate;          // WriteGatePipeline, WriteMode
pub mod validation_gate;     // ValidationPipeline, ValidationGate
pub mod workspace;           // Workspace::open
pub mod git;                 // GitRepo, WorktreeManager, GitCoordinator
pub mod iteration;           // IterationEngine
pub mod session_state;       // SessionState, load/save/index
pub mod path_pattern;        // matches, patterns_overlap
pub mod scope_enforcer;      // FileScope permission checks
pub mod observer;            // WriteGateObserver, AcpObserver, SwarmObserver
pub mod types;               // domain types
pub mod tree_sitter;         // LANGUAGE_REGISTRY, enrich_hunks
pub mod diff_engine;         // compute_hunks
pub mod query_loader;        // .scm discovery
pub mod indent;              // compute_indent
pub mod terminal;            // PTY + OSC 133
pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};

// gaviero-dsl
pub fn compile(source, filename, workflow, runtime_prompt)
    -> Result<CompiledPlan, miette::Report>;
pub fn compile_with_vars(source, filename, workflow, runtime_prompt,
                         override_vars: &[(String, String)])
    -> Result<CompiledPlan, miette::Report>;
```

Binaries (`gaviero`, `gaviero-cli`, `gaviero-mcp-shim`) expose no library API.

---

See [CLAUDE.md](CLAUDE.md) for conventions, build, and rules.
