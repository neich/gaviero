# gaviero-core — Architecture

Shared execution layer. All runtime logic: swarm, memory, MCP, ACP / agent sessions (incl. DeepSeek `tool_agent`), write gate, validation, git, terminal, repo-map, skills, context planning. **No UI or DSL dependencies.**

Conventions and rules: [CLAUDE.md](CLAUDE.md). Workspace topology: [../../ARCHITECTURE.md](../../ARCHITECTURE.md).

---

## Topology

```
gaviero-core (lib, 25 pub mods)
 ├── swarm/ + agent_session/     orchestration + provider transport
 ├── memory/ + mcp/              scoped store + read-only MCP server
 ├── write_gate/ + scope_*       single write path for all agents
 ├── repo_map/ + skills/         graph / topology / skill catalog
 └── acp/ git/ terminal/ …      supporting subsystems
         │
         ├── re-exports tree-sitter 0.25 (+ tree-sitter-gaviero grammar)
         └── McpEndpoint (.gaviero/mcp.sock | \\.\pipe\gaviero-<hash>)
                   ▲
                   │ gaviero-mcp-shim (stdio bridge; not a crate dep)
         subprocess agents (claude / codex / cursor)
         DeepSeek: in-process tool_agent — no shim
```

Depends on: tokio, tree-sitter 0.25 (+ grammars), git2, rusqlite + sqlite-vec, ort + tokenizers, petgraph, portable-pty, rmcp, … — see [CLAUDE.md](CLAUDE.md) Dependencies. Downstream crates must **not** depend on `tree-sitter` directly; use re-exports from [`src/lib.rs`](src/lib.rs).

---

## Modules

**25 pub mods** from [`src/lib.rs`](src/lib.rs):

```
gaviero-core/src/
├─ lib.rs                 Re-exports tree-sitter types + 25 pub mods
├─ types.rs               FileScope, WriteProposal, ModelTier, PrivacyLevel, …
├─ workspace.rs           Workspace::single_folder / load, settings cascade
├─ session_state.rs       SessionState, TabState, StoredConversation, index
├─ tree_sitter.rs         LANGUAGE_REGISTRY (16 langs), enrich_hunks
├─ diff_engine.rs         compute_hunks
├─ write_gate.rs          WriteGatePipeline, WriteMode, proposal lifecycle
├─ observer.rs            WriteGateObserver, AcpObserver, SwarmObserver
├─ scope_enforcer.rs      FileScope checks → path_pattern
├─ path_pattern.rs        Glob matcher + patterns_overlap
├─ git.rs                 GitRepo, WorktreeManager, GitCoordinator (git2)
├─ git_conflict.rs        ConflictRegion / marker parsing (TUI F8/F9)
├─ query_loader.rs        Tree-sitter .scm discovery
├─ skills/                Frontmatter, catalog, planner ResolvedSkill seam
│  ├─ mod.rs / frontmatter.rs / catalog.rs / template.rs
├─ util/                  Shared helpers
│  ├─ fs.rs               Filesystem helpers
│  └─ spawn.rs            Process spawn (+ Windows Job Objects)
├─ indent/                compute_indent (tree-sitter / hybrid / bracket)
├─ terminal/              PTY (portable-pty) + OSC 133
├─ repo_map/              Code graph + ranking
│  ├─ builder.rs / edges.rs / graph_builder.rs / page_rank.rs / store.rs
│  ├─ topology.rs         Shallow folder map → <repo_topology>
│  ├─ symbol_enrichment.rs rustdoc-JSON sidecar (gaviero-cli --graph --enrich)
│  └─ symbol_search.rs    Semantic search over enriched symbols
├─ acp/                   Legacy Claude NDJSON transport
│  ├─ session.rs / protocol.rs / client.rs / factory.rs
├─ agent_session/         V9 AgentSession trait + impls
│  ├─ mod.rs              Turn, TransportContext, build_turn, LegacyAgentSession
│  ├─ claude.rs / codex_exec.rs / codex_app_server.rs / cursor.rs / ollama.rs
│  ├─ registry.rs         SessionConstruction by ProviderProfile
│  └─ tool_agent/         In-process API harness (deepseek: today)
│     ├─ mod.rs / client.rs / config.rs / agent_loop.rs / policy.rs
│     ├─ replay.rs / snapshot.rs / swarm.rs
│     └─ tools/           read / write / bash / glob / grep
├─ context_planner/       Bootstrap / delta / replay → PlannerSelections
├─ mcp/                   In-process MCP server (read-only)
│  ├─ server.rs           spawn_mcp_server, GavieroMcpServer
│  ├─ tools.rs            Seven tools (see MCP below)
│  ├─ transport.rs        McpEndpoint (Unix socket / Windows named pipe)
│  ├─ config_synth.rs     Per-worktree .mcp.json / .codex/ / .cursor/
│  ├─ preflight.rs        Shim PATH + URL checks
│  ├─ telemetry_sink.rs   NDJSON call metrics (--mcp-stats)
│  ├─ resolver.rs         Endpoint / config path resolution
│  ├─ external_memory.rs  Competing memory-MCP detection
│  └─ observer.rs         McpToolCallObserver
├─ memory/                Multi-DB store, writer task, RRF retrieval, eval
├─ swarm/                 Six-phase pipeline + backends
│  └─ backend/            AgentBackend + UnifiedStreamEvent
│     ├─ claude_code.rs / codex.rs / cursor.rs / ollama.rs
│     ├─ deepseek.rs      DeepseekBackend → tool_agent
│     ├─ mock.rs / executor.rs / runner.rs / shared.rs
│     └─ mod.rs           BackendConfig (incl. Deepseek)
├─ iteration/             IterationEngine (retry, BestOfN, TDD)
└─ validation_gate/       TreeSitterGate + CargoCheckGate
```

---

## Abstractions

### `FileScope` ([`types.rs`](src/types.rs))

`owned_paths`, `read_only_paths`, `interface_contracts`. Matched by [`path_pattern::matches`](src/path_pattern.rs); pairwise overlap via [`patterns_overlap`](src/path_pattern.rs) (glob-disjoint siblings ok; prefix/subdir overlaps flagged).

### `WorkUnit` / `CompiledPlan`

- [`WorkUnit`](src/swarm/models.rs) — scope, client/tier/privacy, retries, memory routing, context expansion.
- [`CompiledPlan`](src/swarm/plan.rs) — `DiGraph<PlanNode, DependencyEdge>` + iteration / verification / loop config. `work_units_ordered()`, `hash()` for checkpoints. Produced by `gaviero-dsl` or [`coordinator::plan_coordinated`](src/swarm/coordinator.rs); consumed by [`pipeline::execute`](src/swarm/pipeline.rs).

### `AgentBackend` + `UnifiedStreamEvent` ([`swarm/backend/mod.rs`](src/swarm/backend/mod.rs))

```rust
async fn stream_completion(&self, req: CompletionRequest)
    -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;
```

Events: `TextDelta | ThinkingDelta | ToolCallStart/Delta/End | FileBlock | PathsModified | Usage | Error | Done`.

[`BackendConfig`](src/swarm/backend/mod.rs): `ClaudeCode | Codex | Cursor | Ollama | Deepseek | Custom` (Custom unimplemented). Materialized by [`create_backend`](src/swarm/backend/mod.rs).

**Model spec** ([`shared.rs`](src/swarm/backend/shared.rs)):

| Spec | Backend |
|---|---|
| `claude:<name>` | `ClaudeCodeBackend` |
| `codex:<name>` | `CodexBackend` |
| `cursor:<name>` | `CursorBackend` |
| `ollama:<name>` / `local:<name>` | `OllamaStreamBackend` |
| `deepseek:<name>` | `DeepseekBackend` → [`tool_agent`](src/agent_session/tool_agent) |

[`validate_model_spec`](src/swarm/backend/shared.rs) rejects bare names. Prefixes: `SUPPORTED_PROVIDER_PREFIXES` = `claude`, `codex`, `cursor`, `ollama`, `local`, `deepseek`. DeepSeek ids: `DEEPSEEK_API_MODELS` (`deepseek-v4-pro`, `deepseek-v4-flash`).

### `AgentSession` + `Turn` ([`agent_session/mod.rs`](src/agent_session/mod.rs))

[`build_turn`](src/agent_session/mod.rs) lifts [`PlannerSelections`](src/context_planner/types.rs) into a `Turn`. [`registry::create_session`](src/agent_session/registry.rs) routes by `ProviderProfile`:

| Provider | Continuity | Session |
|---|---|---|
| `claude` | NativeResume | `ClaudeSession` |
| `cursor` | NativeResume | `CursorSession` |
| `codex` | StatelessReplay | `CodexExecSession` |
| `codex-app` | ProcessBound | `CodexAppServerSession` |
| `ollama` / `local` | StatelessReplay | `OllamaSession` |
| `deepseek` | StatelessReplay | `ToolAgentSession` |

Writes: native edit tools (Claude/Codex/Cursor) or Option-B `<file>` blocks (Ollama / DeepSeek) → same [`WriteGatePipeline`](src/write_gate.rs).

### Memory / MCP / observers

- [`MemoryStores`](src/memory/stores.rs) + [`WriterHandle`](src/memory/writer.rs) — multi-DB; single writer task.
- [`GavieroMcpServer`](src/mcp/server.rs) — seven read-only tools; **no `WriterHandle`**.
- Observers in [`observer.rs`](src/observer.rs) / [`memory/observer.rs`](src/memory/observer.rs) / [`mcp/observer.rs`](src/mcp/observer.rs).

---

## Data Flow

### Swarm (6 phases) — [`pipeline::execute`](src/swarm/pipeline.rs)

```
VALIDATE   pairwise scope overlap, Kahn topo-sort, SwarmContextBundle
EXECUTE    worktree + MCP config synth → IterationEngine →
             ContextPlanner::plan → Turn → AgentSession::send_turn →
             FileBlock / tool writes → WriteGatePipeline
MERGE      git merge --no-ff; MergeResolver on conflict
VERIFY     Structural | DiffReview | TestSuite | Combined
CLEANUP    teardown worktrees / gaviero/* branches / MCP configs
CONSOLIDATE Consolidator + TierStats → memory
```

### Write proposal

```
UnifiedStreamEvent::FileBlock (or Cursor snapshot+revert / tool_agent PathsModified)
  → BRIEF LOCK  scope_enforcer / path_pattern
  → NO LOCK     diff_engine::compute_hunks + tree_sitter::enrich_hunks
  → BRIEF LOCK  WriteGatePipeline::insert_proposal (Interactive|AutoAccept|Deferred|RejectAll)
  → NO LOCK     fs::write when finalized
```

### Memory write + retrieve

```
WRITE     WriterHandle::send → writer task: embed (no lock) → brief DB lock → optional ack (ACK_TIMEOUT_MS, 30s)
RETRIEVE  retrieve_ranked: embed → MemoryStores::search_scoped (merged RRF default | cascade kill-switch)
          → score → optional rerank → injection_manifests
```

DBs: global `~/.config/gaviero/memory.db`; workspace+run `<workspace>/.gaviero/memory.db`; repo+module `<folder>/.gaviero/memory.db`.

### MCP

[`spawn_mcp_server`](src/mcp/server.rs) binds [`McpEndpoint`](src/mcp/transport.rs). Subprocess agents use `gaviero-mcp-shim`. Tools ([`tools.rs`](src/mcp/tools.rs)): `memory_search`, `memory_get`, `blast_radius`, `node_doc`, `repo_outline`, `symbol_search`, `symbol_doc` (last two gated by `repoMap.symbolEnrichment.enabled`).

### Two-layer graph context

First turn: `<repo_topology>` ([`topology::build_folder_topology`](src/repo_map/topology.rs)) + `<repo_outline>` (PageRank). `/lite` keeps topology only.

---

## Concurrency

| Component | Primitive | Rule |
|---|---|---|
| WriteGatePipeline | `tokio::sync::Mutex` | No lock across diff / parse / I/O |
| MemoryStore (×N) | `Mutex<Connection>` per DB | Embed outside lock; writes via writer task |
| MCP graph cache | `Mutex<GraphStore>` | Lazy init; serializes blast_radius |
| Writer task | `mpsc` + optional oneshot | Single consumer |
| AgentBus | `broadcast` | Lock-free |
| Parallel agents | `Semaphore` | Bounded per tier |

**Never hold a Mutex across `.await`, tree-sitter parse, or `fs` I/O.** Enforced in [`memory/writer.rs`](src/memory/writer.rs) via `#![deny(clippy::await_holding_lock)]`.

---

## Error Handling

| Error | Handling |
|---|---|
| Scope violation | Reject proposal; observer; no retry |
| Agent failure | `AgentStatus::Failed` → escalate / replan stub |
| Validation gate | Corrective feedback → retry |
| Merge conflict | Claude resolver or user |
| Memory init | Non-fatal `Option<Arc<MemoryStores>>` |
| C1 migration pending | Refuse open until consent |
| MCP bind failure | Log; agents fall back |
| Cursor argv overflow | Explicit reject (~96 KB) |

---

## API

```rust
// crates/gaviero-core/src/lib.rs — 25 pub mods
pub mod acp;
pub mod agent_session;   // + tool_agent (deepseek:)
pub mod context_planner;
pub mod diff_engine;
pub mod git;
pub mod git_conflict;
pub mod indent;
pub mod iteration;
pub mod mcp;
pub mod memory;
pub mod observer;
pub mod path_pattern;
pub mod query_loader;
pub mod repo_map;        // + topology, symbol_enrichment, symbol_search
pub mod scope_enforcer;
pub mod session_state;
pub mod skills;
pub mod swarm;           // backends incl. Deepseek
pub mod terminal;
pub mod tree_sitter;
pub mod types;
pub mod util;
pub mod validation_gate;
pub mod workspace;
pub mod write_gate;

pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};
```

Hard constraints (Write Gate, MCP read-only, explicit `WriteScope`, `provider:model` specs, no UI/DSL deps): see [CLAUDE.md](CLAUDE.md) Rules.
