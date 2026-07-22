# Gaviero — Architecture

Terminal editor + headless CLI for AI agent orchestration. Rust 2024.

**Binaries:** `gaviero` (TUI), `gaviero-cli` (headless), `gaviero-mcp-shim` (stdio↔endpoint bridge)
**Build:** `cargo build` — see [CLAUDE.md](CLAUDE.md).
**Per-crate depth:** each `crates/<crate>/ARCHITECTURE.md`.

---

## Topology

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
   │           │ tui+cli → core+dsl      │              │
   │           │ dsl → core              │              │
   │           │ shim: standalone        │              │
   │           ▼                         │              │
   │    McpEndpoint ◄────────────────────┘              │
   │    .gaviero/mcp.sock | \\.\pipe\gaviero-<hash>     │
   └─── re-exports tree-sitter ─────────────────────────┘
```

| Crate | Type | Role | Detail |
|---|---|---|---|
| [`gaviero-core`](crates/gaviero-core/) | lib (**25** pub mods) | Swarm, memory, MCP, ACP/agent-session (+ DeepSeek `tool_agent`), write gate, repo-map, skills | [ARCHITECTURE](crates/gaviero-core/ARCHITECTURE.md) |
| [`gaviero-tui`](crates/gaviero-tui/) | bin `gaviero` | Ratatui UI, observers, slash commands | [ARCHITECTURE](crates/gaviero-tui/ARCHITECTURE.md) |
| [`gaviero-cli`](crates/gaviero-cli/) | bin `gaviero-cli` | Clap runner (~4000-line `main.rs`), eval/memory admin | [ARCHITECTURE](crates/gaviero-cli/ARCHITECTURE.md) |
| [`gaviero-dsl`](crates/gaviero-dsl/) | lib (**9** pub mods) | `.gaviero` compiler → `CompiledPlan` | [ARCHITECTURE](crates/gaviero-dsl/ARCHITECTURE.md) |
| [`gaviero-mcp-shim`](crates/gaviero-mcp-shim/) | bin | stdio↔`McpEndpoint` bridge; zero workspace deps | [ARCHITECTURE](crates/gaviero-mcp-shim/ARCHITECTURE.md) |
| [`tree-sitter-gaviero`](crates/tree-sitter-gaviero/) | grammar | Editor syntax tree for `.gaviero` | [ARCHITECTURE](crates/tree-sitter-gaviero/ARCHITECTURE.md) |

**Dependency rules:** core has no UI/DSL deps. tui + cli depend on core + dsl. dsl depends on core. mcp-shim is self-contained and reaches core only over the MCP endpoint. Downstream crates must not depend on `tree-sitter` directly — use [`gaviero_core`](crates/gaviero-core/src/lib.rs) re-exports.

---

## Modules

### Core (25) — [`lib.rs`](crates/gaviero-core/src/lib.rs)

`acp`, `agent_session` (+ `tool_agent/`), `context_planner`, `diff_engine`, `git`, `git_conflict`, `indent`, `iteration`, `mcp`, `memory`, `observer`, `path_pattern`, `query_loader`, `repo_map` (+ topology / symbol enrichment), `scope_enforcer`, `session_state`, `skills`, `swarm` (+ backends incl. `deepseek`), `terminal`, `tree_sitter`, `types`, `util`, `validation_gate`, `workspace`, `write_gate`.

### DSL (9) — [`lib.rs`](crates/gaviero-dsl/src/lib.rs)

`ast`, `compiler`, `reviewers`, `workflow_params`, `error`, `lexer`, `parser`, `resolver`, `tiers`.

### Other crates

- TUI: `app/`, `editor/` (wrap, diff), `panels/`, `platform.rs` — [tui ARCHITECTURE](crates/gaviero-tui/ARCHITECTURE.md).
- CLI: single [`main.rs`](crates/gaviero-cli/src/main.rs) (~4000 lines); `Cli` is authoritative for flags (no `--no-memory`).
- Shim: [`main.rs`](crates/gaviero-mcp-shim/src/main.rs) (~187 lines).
- Grammar: `grammar.js` → generated `parser.c`.

---

## Abstractions

### Scope + plan

- [`FileScope`](crates/gaviero-core/src/types.rs) — glob owned/read-only paths; overlap via [`path_pattern::patterns_overlap`](crates/gaviero-core/src/path_pattern.rs).
- [`WorkUnit`](crates/gaviero-core/src/swarm/models.rs) / [`CompiledPlan`](crates/gaviero-core/src/swarm/plan.rs) — DAG + iteration/verify/loop config. From DSL [`compile_file`](crates/gaviero-dsl/src/lib.rs) or [`coordinator::plan_coordinated`](crates/gaviero-core/src/swarm/coordinator.rs).

### Provider transport

- [`AgentBackend`](crates/gaviero-core/src/swarm/backend/mod.rs) + [`UnifiedStreamEvent`](crates/gaviero-core/src/swarm/backend/mod.rs) — ClaudeCode, Codex, Cursor, Ollama, **Deepseek**, Mock.
- [`AgentSession`](crates/gaviero-core/src/agent_session/mod.rs) + [`Turn`](crates/gaviero-core/src/agent_session/mod.rs) — claude, codex_exec, codex_app_server, cursor, ollama, **tool_agent** (DeepSeek), registry.
- Model spec: `provider:model`. Prefixes in [`SUPPORTED_PROVIDER_PREFIXES`](crates/gaviero-core/src/swarm/backend/shared.rs): `claude`, `codex`, `cursor`, `ollama`, `local`, `deepseek`. Bare names rejected by [`validate_model_spec`](crates/gaviero-core/src/swarm/backend/shared.rs).

### Memory + MCP + write path

- [`MemoryStores`](crates/gaviero-core/src/memory/stores.rs) + [`WriterHandle`](crates/gaviero-core/src/memory/writer.rs) — multi-DB; single writer task.
- [`GavieroMcpServer`](crates/gaviero-core/src/mcp/server.rs) — seven read-only tools over [`McpEndpoint`](crates/gaviero-core/src/mcp/transport.rs); subprocess agents use `gaviero-mcp-shim`. DeepSeek does not.
- [`WriteGatePipeline`](crates/gaviero-core/src/write_gate.rs) — every agent file change.

### Observers

[`WriteGateObserver`](crates/gaviero-core/src/observer.rs), [`AcpObserver`](crates/gaviero-core/src/observer.rs), [`SwarmObserver`](crates/gaviero-core/src/observer.rs) (+ memory/MCP observers). TUI and CLI implement; core never imports UI types.

---

## Data Flow

### Agent write

```
Agent stream (ACP / Codex / Cursor / Ollama / DeepSeek tool_agent)
  → UnifiedStreamEvent::FileBlock | native tool write | PathsModified
  → scope check (brief lock) → diff + enrich (no lock)
  → WriteGatePipeline::insert_proposal → fs::write when finalized
```

### Swarm — [`pipeline::execute`](crates/gaviero-core/src/swarm/pipeline.rs)

`VALIDATE → EXECUTE → MERGE → VERIFY → CLEANUP → CONSOLIDATE`
Per unit: git worktree + MCP config synth → `ContextPlanner` → `AgentSession` → Write Gate → validation retries. Detail: [core ARCHITECTURE](crates/gaviero-core/ARCHITECTURE.md).

### Memory

Writes: `WriterHandle` → embed outside lock → brief DB insert.
Retrieval: [`retrieve_ranked`](crates/gaviero-core/src/memory/retrieval.rs) (merged RRF default).
Two-layer graph: `<repo_topology>` + `<repo_outline>`; TUI `/lite` drops outline/memory/impact.

---

## Concurrency

Shared tokio runtime.

| Subsystem | Rule |
|---|---|
| Write gate / memory SQLite | Brief `Mutex`; never across await / parse / fs / embed |
| Memory writes | Single writer task only |
| TUI | One `mpsc` event channel; only main loop mutates `App` |
| Parallel agents | `Semaphore` per tier |

Enforced in writer via `#![deny(clippy::await_holding_lock)]`.

---

## Error Handling

| Layer | Strategy |
|---|---|
| DSL | `miette::Report` + source spans |
| Write scope | Reject proposal; observer |
| Agent / validation | Retry / escalate |
| Memory init | Non-fatal `Option` |
| C1 migration | Consent required (CLI `--accept-c1-migration` / TUI prompt) |
| MCP bind | Log; fall back |

CLI exit codes: 0 success, 1 failure, 2 args, 3 setup — [cli ARCHITECTURE](crates/gaviero-cli/ARCHITECTURE.md).

---

## API

```rust
// gaviero-core — 25 pub mods (crates/gaviero-core/src/lib.rs)
pub mod acp; pub mod agent_session; pub mod context_planner;
pub mod diff_engine; pub mod git; pub mod git_conflict;
pub mod indent; pub mod iteration; pub mod mcp; pub mod memory;
pub mod observer; pub mod path_pattern; pub mod query_loader;
pub mod repo_map; pub mod scope_enforcer; pub mod session_state;
pub mod skills; pub mod swarm; pub mod terminal; pub mod tree_sitter;
pub mod types; pub mod util; pub mod validation_gate;
pub mod workspace; pub mod write_gate;
pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};

// gaviero-dsl
pub fn compile(...) -> Result<CompiledPlan, miette::Report>;
pub fn compile_with_vars(..., override_vars, override_tiers, override_params) -> …;
pub fn compile_file(..., override_vars, override_tiers, override_params) -> …;
pub fn workflow_execution_mode(...) -> Result<ExecutionMode, miette::Report>;
pub use tiers::load_tier_overrides;
```

Binaries expose no library API.

**Hard constraints:** Write Gate for all agent writes; MCP read-only; explicit `WriteScope`; `provider:model` specs; no Mutex across I/O/await; core free of UI/DSL types — [CLAUDE.md](CLAUDE.md).
