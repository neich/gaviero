# gaviero-core

## Overview

Core runtime library for Gaviero. All execution logic — agent orchestration, write gates, memory, git, terminal — lives here. The TUI (`gaviero-tui`), CLI (`gaviero-cli`), and DSL compiler (`gaviero-dsl`) are thin frontends that delegate to this library. The crate has **no UI dependencies**.

Subsystems: chat execution (ACP + provider dispatch), swarm orchestration, Write Gate, iteration/validation, semantic memory, in-process MCP server (read-only), repo map (topology + PageRank outline), git/worktrees, workspace settings, terminal PTY, and skills catalog.

## Installation

```bash
cargo build  -p gaviero-core
cargo test   -p gaviero-core
cargo clippy -p gaviero-core
```

Most tests run offline. Network/model tests (Ollama, embedder downloads, provider CLI presence) are marked `#[ignore]`.

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

### Coordinated planning

```rust
let plan = pipeline::plan_coordinated(&task, &context).await?;
println!("{}", plan.to_gaviero_script()?);
```

## Examples

**Provider model strings** — every spec requires a `provider:model` prefix; bare names are rejected by `validate_model_spec`:

| Provider | Examples | Notes |
|---|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus` | Subprocess (Claude Code) |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` | Subprocess (exec / app-server) |
| Cursor | `cursor:claude-4-sonnet` | Subprocess (Cursor CLI) |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:model-name` | Local server |
| DeepSeek | `deepseek:deepseek-v4-pro` | In-process HTTP (`tool_agent`) |

**Observer traits** — implement to receive execution events:

```rust
// observer::WriteGateObserver  — proposal changes
// observer::AcpObserver       — agent chat events
// observer::SwarmObserver     — multi-agent coordination events
```

## Configuration

Reads `.gaviero/settings.json` (cascade documented in [gaviero-tui](../gaviero-tui/README.md#configuration)):

| Setting | Default | Effect |
|---|---|---|
| `agent.model` | `claude:sonnet` | Default execution model spec |
| `agent.ollamaBaseUrl` | `http://localhost:11434` | Ollama server URL |
| `memory.embedder.model` | `nomic` | Embedder: `nomic`, `gte-modernbert`, `jina-code`, `e5-small-v2`, or `dual:<a>,<b>` |
| `memory.retrieval.mode` | merged | `merged` (multi-scope RRF) or `cascade` (legacy) |
| `memory.reranker.enabled` | `true` | Cross-encoder reranker |
| `repoMap.symbolEnrichment.enabled` | `false` | Enables `symbol_search` / `symbol_doc` MCP tools |

## API

### Primary entry points

| Subsystem | Entry point | Purpose |
|---|---|---|
| Chat | `acp::client::AcpPipeline` | Single-turn agent execution |
| Swarm | `swarm::pipeline::execute()` | Multi-agent orchestration |
| Planning | `swarm::pipeline::plan_coordinated()` | Generate reviewable `.gaviero` plans |
| Backend | `swarm::backend::AgentBackend` | Provider abstraction |
| Routing | `swarm::router::TierRouter` | Model tier resolution |
| Iteration | `iteration::IterationEngine` | Retry loops with verification |
| Write Gate | `write_gate::WriteGatePipeline` | Diff review + file application |
| Validation | `validation_gate::ValidationGate` | Syntax/compilation checks |
| Memory | `memory::MemoryStore` | Scoped semantic embeddings |
| Workspace | `workspace::Workspace` | Settings and namespace resolution |
| Git | `git::{GitRepo, WorktreeManager}` | Repository and worktree operations |
| Terminal | `terminal::TerminalManager` | PTY lifecycle |

### Module map (25 public modules)

| Module | Purpose |
|---|---|
| `acp/` | Claude subprocess protocol, session factory, file-block routing |
| `agent_session/` | Per-provider sessions: `claude`, `codex_exec`, `codex_app_server`, `cursor`, `ollama`, `tool_agent` (DeepSeek), `registry` |
| `swarm/` | Orchestration, tier routing, DAG execution, backends, verification, git merge |
| `mcp/` | In-process MCP server (seven read-only tools), config synthesis, transport |
| `memory/` | Five-level scoped embeddings, RRF retrieval, consolidation, soft-delete |
| `write_gate/` | Diff review, hunk acceptance, scope enforcement |
| `validation_gate/` | tree-sitter syntax, cargo compile, test verification |
| `iteration/` | Retry loops, escalation, best-of-N |
| `context_planner/` | Repo-map queries, `callers_of`, `tests_for`, compaction |
| `repo_map/` | PageRank outline, code graph, `topology.rs`, symbol enrichment |
| `skills/` | Turn-scoped instruction templates (frontmatter, catalog) |
| `session_state/` | Persistent session state, checkpoint/resume |
| `workspace/` | Settings cascade, namespace resolution |
| `git/` | `git2` wrapper, worktree management, merge |
| `git_conflict.rs` | Merge conflict resolution helpers |
| `terminal/` | PTY lifecycle, OSC 133, `vt100` emulation |
| `tree_sitter/` | Language registry (16 langs), query loader |
| `diff_engine/` | Hunk computation, context extraction |
| `scope_enforcer/` | File path validation, write boundaries |
| `path_pattern/` | Glob matching, scope overlap detection |
| `query_loader/` | tree-sitter query loading |
| `observer/` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` |
| `indent/` | Smart indentation strategies |
| `util/` | Shared filesystem and spawn helpers |
| `types/` | `FileScope`, `WriteProposal`, `ModelTier`, … |

`tree-sitter` types are re-exported from this crate; downstream crates must not depend on `tree-sitter` directly.

### Design invariants

- No `Mutex` held across I/O, embedding, or parsing; the memory writer task owns SQLite writes.
- Memory writes require an explicit `WriteScope`.
- MCP tools are read-only by construction — no write tools for subprocess agents.

## See Also

- [CLAUDE.md](CLAUDE.md) — internal conventions and invariants
- [ARCHITECTURE.md](ARCHITECTURE.md) — module dependency graph, pipelines
- [Root README](../../README.md) — user-facing feature overview

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
