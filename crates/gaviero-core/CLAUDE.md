# gaviero-core

All runtime logic: swarm orchestration, memory, MCP server, write gate, scope/validation gates, agent sessions, git, terminal, repo-map. No UI or DSL dependencies.

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
```

Network/model tests (Ollama, embedder downloads) are `#[ignore]` by default.

## Public Modules (from `src/lib.rs`, 22)

| Module | Purpose |
|---|---|
| `swarm` | Multi-agent orchestration: tier routing, parallel fan-out, verification, merge, replan, calibration, context bundling |
| `memory` | Multi-DB ONNX-embedding store + three-cadence consolidation |
| `mcp` | In-process MCP server (read-only tools for subprocess agents) |
| `acp` | Claude subprocess sessions (legacy ACP transport: persistent / one-shot) |
| `agent_session` | Backend-neutral agent session transport (claude, codex_exec, codex_app_server, ollama, registry) |
| `context_planner` | Pre-prompt context assembly (graph + memory + chat history + ledger + compaction) |
| `write_gate` | File-modification boundary, diff review, scope validation, observer callbacks |
| `validation_gate` | Structural + semantic post-write validation pipeline |
| `scope_enforcer` | `FileScope` enforcement on agent proposals |
| `path_pattern` | Glob-aware scope overlap detection (backs DSL scope validation) |
| `session_state` | Checkpointable session state for resume |
| `repo_map` | Code knowledge graph (tree-sitter) + shallow `topology.rs` for `<repo_topology>` |
| `tree_sitter` | 16-language registry, query loader |
| `diff_engine` | Unified diff generation/application |
| `git` | `git2` wrapper, worktrees, branches |
| `terminal` | PTY + OSC 133 + `vt100` emulation |
| `indent` | Hybrid indent detection |
| `iteration` | Iteration/retry loop control |
| `observer` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` traits |
| `query_loader` | Tree-sitter query discovery |
| `types` | Shared boundary types |
| `workspace` | `.gaviero/settings.json` + workspace discovery |

## Key Subsystems

**Swarm** (`swarm/`): tier execution (local/cheap/expensive/codex), parallel fan-out, verification gates (structural / diff-review / test-runner / combined), scope validation, dependency DAG, checkpoint/resume, merge conflict handling, replanner, calibration, context bundle assembly. Backends in `swarm/backend/`: `claude_code`, `codex`, `ollama`, `mock` — all behind the `AgentBackend` trait.

**Memory** (`memory/`): SQLite + sqlite-vec, pluggable `Embedder` trait. Default `gte-modernbert-base` (768-dim); `nomic-embed-text-v1.5` is selectable but legacy. Multi-DB registry (global / workspace / per-folder) via `stores.rs`. Single-consumer writer task owns all writes (`writer.rs`, `WriterMessage` mpsc) — no callsite holds the SQLite Mutex during embed. Default retrieval is merged multi-scope hybrid (RRF: vector 0.7 + FTS 0.3); legacy narrow→wide cascade with 0.70 early-exit is retained as a kill-switch behind `memory.retrieval.mode = "cascade"`. Optional cross-encoder reranker (`reranker.rs`) off by default. Three-cadence consolidation: per-turn `extractor.rs` → per-session `session_consolidator.rs` → idle/weekly `sleeptime.rs` + `sleeptime_scheduler.rs` (decay sweep, near-dup merge, cross-scope promotion, trust re-scoring, history compression, summary prune). Soft-delete via `/forget` writes to a `deletions` audit table; History rows are immutable except via the C2.4 redaction path. Other modules: `annotations` (turn-level `<turn_annotations>` JSON sidecar), `compression` (zstd + SHA-256 round-trip after `compressAfterDays`, default 90), `eval` (recall@K / MRR regression harness), `trust_defaults`, `telemetry`. Store I/O is split under `store/` (search, write, panel_ops, deletions_ops, compression_ops, sleeptime_ops, telemetry_ops, manifest).

**MCP** (`mcp/`): In-process MCP server exposing three read-only tools to subprocess coding agents — `memory_search`, `blast_radius`, `node_doc`. Listens on `<workspace>/.gaviero/mcp.sock`; subprocess agents reach it via the `gaviero-mcp-shim` binary. `config_synth.rs` writes the `.mcp.json` entries that point at the shim; `external_memory.rs` detects and disables competing memory MCP servers; `observer.rs` surfaces tool-call events to the host process. Read-only by construction.

**Agent session** (`agent_session/`): backend-neutral transport. Codex is dual-mode — `codex_exec.rs` for one-shot exec mode and `codex_app_server.rs` for persistent app-server mode; the `registry.rs` resolver picks based on model spec.

**ACP** (`acp/`): legacy direct Claude subprocess path — session factory, argv/tempfile prompt spill, streaming file-block extraction. Newer code paths go through `agent_session::claude` instead.

**Write Gate** (`write_gate/`): interactive / auto-accept / reject-all modes; diff review; scope validation; observer callbacks.

## Conventions

- Lock discipline: never hold Mutex across I/O, parsing, or embedding.
- `AgentBackend` trait is object-safe; all backends in `swarm/backend/` implement it.
- Embedder is pluggable (`Embedder` trait + `model_manager::resolve_embedder_model`); default `gte-modernbert-base` (768-dim). Cosine similarity.
- Memory writes require explicit `WriteScope` — never infer. All writes flow through the writer task.
- Scoring: 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights. B4 recency floor + decay-exempt types (Decision/Convention/Invariant/Preference) protect reference memories from age-out.
- MCP tools are read-only. Never add a write tool to `mcp/`; route writes through the Write Gate or the memory writer task.

## Dependencies

- `tree-sitter 0.25` + 16 grammars
- `git2 0.19`
- `rusqlite 0.32` (bundled) + `sqlite-vec`
- `ort 2.0.0-rc.12` + `tokenizers` (ONNX inference)
- `petgraph 0.8` (DAG ops)
- `portable-pty` + `vt100` (terminal emulation)

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — module map, swarm/memory pipelines, data flow
- [../../ARCHITECTURE.md](../../ARCHITECTURE.md) — workspace-wide design
