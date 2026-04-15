# gaviero-core

Core runtime library for Gaviero. All execution logic — agent orchestration, write gates, memory, git, terminal — lives here. The TUI (`gaviero-tui`), CLI (`gaviero-cli`), and DSL compiler (`gaviero-dsl`) are all frontends that delegate to this library.

## Overview

`gaviero-core` provides the complete execution engine for AI-powered code workflows:

- **Chat execution** — Claude subprocess protocol (ACP) + provider-aware agent dispatch
- **Swarm orchestration** — Multi-agent coordination with tier routing, scoped execution, and dependency graphs
- **Write gates** — Diff review and interactive acceptance before changes touch disk
- **Iteration & validation** — Retry loops with syntax checking, compilation, and test-based verification
- **Semantic memory** — Hierarchical scoped embeddings (ONNX, SQLite) for cross-session knowledge
- **Git & worktrees** — Repository operations and isolated execution contexts
- **Workspace settings** — Configuration cascade (project → user → defaults)
- **Terminal management** — PTY lifecycle and interactive shell sessions

There are no UI dependencies. Both TUI and CLI use the public APIs in this crate.

## Installation & Build

```bash
cargo build -p gaviero-core
cargo test -p gaviero-core
cargo clippy -p gaviero-core
```

Most tests run offline. Some (marked `#[ignore]`) require network for Ollama health checks or model downloads.

## Provider Model Strings

Model selection uses a unified convention across chat and swarm execution:

- **Claude models** — `sonnet`, `opus`, `haiku` (shorthand) or `claude:sonnet`, `claude-code:haiku` (explicit)
- **Ollama/local** — `ollama:qwen2.5-coder:7b` or `local:model-name`
- **Default** — unprefixed names route to Claude for backward compatibility

Ollama server URL is configured via `SwarmConfig.ollama_base_url` or workspace setting `agent.ollamaBaseUrl`.

## API Overview

### Primary Entry Points

| Subsystem | Main Type/Function | Purpose |
|---|---|---|
| **Chat** | `acp::client::AcpPipeline` | Single-turn agent execution with prompt enrichment |
| **Swarm** | `swarm::pipeline::execute()` | Multi-agent orchestration from compiled plans |
| **Planning** | `swarm::pipeline::plan_coordinated()` | Generate reviewable `.gaviero` plans |
| **Backend** | `swarm::backend::AgentBackend` trait | Provider abstraction (Claude, Ollama, mock) |
| **Routing** | `swarm::router::TierRouter` | Model tier resolution (local/cheap/expensive) |
| **Iteration** | `iteration::IterationEngine` | Retry loops with verification feedback |
| **Write Gate** | `write_gate::WriteGatePipeline` | Diff review + file application |
| **Validation** | `validation_gate::ValidationGate` trait | Syntax and compilation verification |
| **Memory** | `memory::MemoryStore` | Scoped semantic embeddings |
| **Workspace** | `workspace::Workspace` | Settings and namespace resolution |
| **Git** | `git::{GitRepo, WorktreeManager}` | Repository and worktree operations |
| **Terminal** | `terminal::TerminalManager` | PTY lifecycle and shell sessions |

### Observation & Events

Implement observer traits to receive execution events:
- `observer::WriteGateObserver` — proposal changes
- `observer::AcpObserver` — agent chat events
- `observer::SwarmObserver` — multi-agent coordination events

## Usage Examples

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
println!("{}", plan.to_gaviero_script()?);  // Reviewable .gaviero format
```

## Module Overview

| Module | Purpose |
|---|---|
| `acp/` | Claude subprocess protocol (ACP), session factory, prompt enrichment, file block routing |
| `swarm/` | Multi-agent orchestration, tier routing, DAG execution, verification, git merge, backends |
| `iteration/` | Retry loops, escalation, best-of-N strategy |
| `validation_gate/` | Syntax validation (tree-sitter), compilation checks (cargo), test verification |
| `write_gate/` | Diff review, hunk acceptance/rejection, scope enforcement |
| `memory/` | 5-level hierarchical scoped embeddings (ONNX + SQLite + RRF search) |
| `repo_map/` | PageRank-based context ranking, code graph, symbol resolution |
| `workspace/` | Settings cascade, namespace resolution, project configuration |
| `git/` | Git2 wrapper, worktree management, merge + conflict resolution |
| `terminal/` | PTY lifecycle, OSC 133 parsing, interactive shell sessions |
| `tree_sitter/` | Language registry (16 langs), query loader, AST enrichment |
| `diff_engine/` | Hunk computation, context extraction |
| `indent/` | Smart indentation (tree-sitter + hybrid + bracket strategies) |
| `scope_enforcer/` | File path validation, write boundary enforcement |

## Design Notes

- **No UI dependencies** — core is pure library code
- **Provider-neutral** — model strings are resolved at runtime, not compile-time
- **Lock discipline** — no Mutex held across I/O, embedding, or parsing
- **Memory writes** — always require explicit `WriteScope`; never infer scope level
- **Scoring** — 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust
- **Cascading search** — narrowest scope first, early exit at 0.70 confidence threshold

## See Also

- [ARCHITECTURE.md](../../ARCHITECTURE.md) — complete module dependency graph, data flow, type signatures
- [crates/gaviero-dsl/README.md](../gaviero-dsl/README.md) — language syntax and examples
- [CLAUDE.md](../../CLAUDE.md) — project conventions and rules
