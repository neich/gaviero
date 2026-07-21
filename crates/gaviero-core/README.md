# gaviero-core

Core runtime library for Gaviero. All execution logic — agent orchestration, write gates, memory, git, terminal — lives here. The TUI (`gaviero-tui`), CLI (`gaviero-cli`), and DSL compiler (`gaviero-dsl`) are thin frontends that delegate to this library.

## Overview

`gaviero-core` provides the complete execution engine for AI-powered code workflows, with **no UI dependencies**:

- **Chat execution** — Claude subprocess protocol (ACP) plus provider-aware agent dispatch (Claude Code, Codex, Cursor, Ollama, DeepSeek).
- **Swarm orchestration** — multi-agent coordination with tier routing, scoped execution, and dependency DAGs.
- **Write gates** — diff review and interactive acceptance before any change touches disk.
- **Iteration & validation** — retry loops with syntax checking, compilation, and test-based verification.
- **Semantic memory** — five-level scoped ONNX embeddings (default `nomic-embed-text-v1.5`, SQLite) with merged multi-scope RRF retrieval, three-cadence consolidation, soft-delete, and optional cross-encoder reranking.
- **In-process MCP server** — read-only tools for subprocess agents over a Unix socket / Windows named pipe.
- **Repo map** — PageRank context ranking plus shallow filesystem topology for the two-layer `<repo_topology>` + `<repo_outline>` bundle.
- **Git & worktrees** — repository operations and isolated execution contexts.
- **Workspace settings** — configuration cascade (project → user → defaults).
- **Terminal** — PTY lifecycle and interactive shell sessions.

## Installation

```bash
cargo build  -p gaviero-core
cargo test   -p gaviero-core
cargo clippy -p gaviero-core
```

Most tests run offline. Network/model tests (Ollama health checks, embedder downloads, Cursor/Codex/Claude CLI presence) are marked `#[ignore]`.

## Usage

### Single-turn chat

```rust
use gaviero_core::acp::client::AcpPipeline;

let pipeline = AcpPipeline::new(workspace);
let response = pipeline.send_prompt("review this code", &file_references)?;
```

### Multi-agent swarm execution

```rust
use gaviero_core::swarm::pipeline;

let result = pipeline::execute(&compiled_plan, &workspace, &swarm_config).await?;
```

### Generating a coordinated plan

```rust
use gaviero_core::swarm::pipeline;

let plan = pipeline::plan_coordinated(&task, &context).await?;
println!("{}", plan.to_gaviero_script()?);  // reviewable .gaviero format
```

## Provider Model Strings

Model selection uses a unified convention across chat and swarm execution. Every spec requires a `provider:model` prefix — bare names are rejected by `validate_model_spec` (`swarm::backend::shared`).

| Provider | Examples | Notes |
|---|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus`, `claude:haiku`, `claude:claude-opus-4-7` | Subprocess (Claude Code) |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` | Subprocess (dual-mode: exec / app-server) |
| Cursor | `cursor:claude-4-sonnet` | Subprocess (Cursor CLI) |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:model-name` | Local server |
| DeepSeek | `deepseek:deepseek-v4-pro`, `deepseek:deepseek-v4-flash` | In-process HTTP API harness (`tool_agent`) |

Ollama server URL is set via `SwarmConfig.ollama_base_url` or workspace setting `agent.ollamaBaseUrl`.

## API

### Primary entry points

| Subsystem | Main type/function | Purpose |
|---|---|---|
| **Chat** | `acp::client::AcpPipeline` | Single-turn agent execution with prompt enrichment |
| **Swarm** | `swarm::pipeline::execute()` | Multi-agent orchestration from compiled plans |
| **Planning** | `swarm::pipeline::plan_coordinated()` | Generate reviewable `.gaviero` plans |
| **Backend** | `swarm::backend::AgentBackend` trait | Provider abstraction (Claude, Codex, Cursor, Ollama, DeepSeek, mock) |
| **Routing** | `swarm::router::TierRouter` | Model tier resolution (local / cheap / expensive / codex / cursor) |
| **Iteration** | `iteration::IterationEngine` | Retry loops with verification feedback |
| **Write Gate** | `write_gate::WriteGatePipeline` | Diff review + file application |
| **Validation** | `validation_gate::ValidationGate` trait | Syntax and compilation verification |
| **Memory** | `memory::MemoryStore` | Scoped semantic embeddings |
| **Workspace** | `workspace::Workspace` | Settings and namespace resolution |
| **Git** | `git::{GitRepo, WorktreeManager}` | Repository and worktree operations |
| **Terminal** | `terminal::TerminalManager` | PTY lifecycle and shell sessions |

### Observation & events

Implement observer traits to receive execution events:

- `observer::WriteGateObserver` — proposal changes
- `observer::AcpObserver` — agent chat events
- `observer::SwarmObserver` — multi-agent coordination events

### Module map

| Module | Purpose |
|---|---|
| `acp/` | Claude subprocess protocol (ACP), session factory, prompt enrichment, file-block routing |
| `agent_session/` | Per-agent session lifecycle: `claude`, `codex_exec`, `codex_app_server`, `cursor`, `ollama`, `tool_agent` (DeepSeek), `registry` |
| `mcp/` | In-process MCP server (read-only tools), config synthesis, external-server detection |
| `swarm/` | Multi-agent orchestration, tier routing, DAG execution, verification, git merge, backends, replanner, calibration, context bundles |
| `skills/` | Turn-scoped instruction templates (frontmatter parse, substitution, multi-root catalog) |
| `context_planner/` | Context selection: repo-map queries, `callers_of`, `tests_for`, chat memory, compaction, ledger |
| `session_state/` | Persistent session state (checkpoint / resume, history) |
| `iteration/` | Retry loops, escalation, best-of-N strategy |
| `validation_gate/` | Syntax validation (tree-sitter), compilation checks (cargo), test verification |
| `write_gate/` | Diff review, hunk acceptance/rejection, scope enforcement |
| `memory/` | Five-level hierarchical scoped embeddings (SQLite + sqlite-vec, RRF hybrid); three-cadence consolidation, soft-delete, multi-DB registry, optional reranker |
| `repo_map/` | PageRank context ranking, code graph, symbol resolution; shallow `topology.rs` for `<repo_topology>` |
| `path_pattern/` | Glob-style path matching and scope overlap detection for DSL validation |
| `workspace/` | Settings cascade, namespace resolution, project configuration |
| `git/` | `git2` wrapper, worktree management, merge + conflict resolution |
| `terminal/` | PTY lifecycle, OSC 133 parsing, `vt100` emulation |
| `tree_sitter/` | Language registry (16 langs), query loader, AST enrichment |
| `diff_engine/` | Hunk computation, context extraction |
| `indent/` | Smart indentation (tree-sitter + hybrid + bracket strategies) |
| `scope_enforcer/` | File path validation, write boundary enforcement |
| `observer/` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` traits |
| `types/` | Shared boundary types (`FileScope`, `WriteProposal`, `ModelTier`, …) |

`tree-sitter` types (`Language`, `Node`, `Parser`, `Query`, `Tree`, …) are re-exported from this crate; downstream crates **must not** depend on the `tree-sitter` crate directly.

## Configuration

The crate reads `.gaviero/settings.json` (see [`gaviero-tui`](../gaviero-tui/README.md#configuration) for the full cascade). Keys that steer core subsystems:

| Setting | Default | Effect |
|---|---|---|
| `agent.model` | `claude:sonnet` | Default execution model spec |
| `agent.ollamaBaseUrl` | `http://localhost:11434` | Ollama server URL |
| `memory.embedder.model` | `nomic` | Memory embedder: `nomic`, `gte-modernbert`, `jina-code`, `e5-small-v2`, or `dual:<a>,<b>` A/B mode |
| `memory.retrieval.mode` | merged | `merged` (multi-scope RRF hybrid) or `cascade` (legacy narrowest-scope-first) |
| `memory.reranker.enabled` | `true` | Cross-encoder reranker (model `minilm`) |
| `repoMap.embedder.model` | `jina-code` | Symbol-vector embedder for `symbol_search` |
| `repoMap.symbolEnrichment.enabled` | `false` | Exposes the `symbol_search` / `symbol_doc` MCP tools |

## Design Notes

- **No UI dependencies** — core is pure library code (`tui-term`/`vt100` are allowed only for the embedded terminal subsystem).
- **Provider-neutral** — model strings are resolved at runtime, not compile time.
- **Lock discipline** — no `Mutex` held across I/O, embedding, or parsing; the memory writer task is the single owner of SQLite writes.
- **Memory writes** — always require an explicit `WriteScope`; scope level is never inferred.
- **Scoring** — 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights; decay-exempt types (`Decision`/`Convention`/`Invariant`/`Preference`) are recency-floored.
- **Retrieval** — merged multi-scope by default (RRF hybrid: vector 0.7 + FTS 0.3 across all scopes simultaneously); cascade with 0.70 early-exit is the legacy kill-switch.
- **MCP tools** — read-only by construction; no write tools are exposed to subprocess agents.

## Dependencies

- `tree-sitter 0.25` + 16 grammars, `git2 0.19`, `rusqlite 0.32` + `sqlite-vec 0.1.8`
- `ort 2.0` + `tokenizers 0.19` + `ndarray 0.17` — ONNX inference
- `petgraph 0.8` (swarm DAG), `portable-pty 0.9` + `vt100 0.16` (terminal), `rmcp 1.5` + `schemars 1.2` (MCP)
- `zstd 0.13` + `bincode 1.3` (history compression), plus `reqwest`, `tokio`, `chrono`, `ropey`, `similar`, and more

## See Also

- [CLAUDE.md](CLAUDE.md) — internal module notes, conventions, and invariants
- [ARCHITECTURE.md](ARCHITECTURE.md) — module dependency graph, swarm/memory pipelines, MCP topology
- [`crates/gaviero-dsl/README.md`](../gaviero-dsl/README.md) — workflow language
- [Root README](../../README.md) — user-facing feature overview

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
