# gaviero-core

All runtime logic: swarm orchestration, memory, MCP server, ACP/agent sessions, write gate, scope + validation gates, git, terminal, repo-map. **No UI or DSL dependencies.**

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
cargo test -p gaviero-core --features api-embedders   # placeholder factory only
```

Network/model tests (Ollama, embedder downloads, Cursor/Codex/Claude CLI presence) are `#[ignore]`.

## Public Modules (22, from [src/lib.rs](src/lib.rs))

| Module | Purpose |
|---|---|
| `swarm` | Multi-agent orchestration: tier routing, parallel fan-out, verification, merge, replan, calibration, context bundling. |
| `memory` | Multi-DB ONNX-embedding store + three-cadence consolidation. |
| `mcp` | In-process MCP server (read-only tools for subprocess agents). |
| `acp` | Legacy direct Claude subprocess transport (one-shot / persistent). |
| `agent_session` | Backend-neutral session transport: `claude`, `codex_exec`, `codex_app_server`, `cursor`, `ollama`, `registry`. |
| `context_planner` | Pre-prompt context assembly (graph + memory + chat history + ledger + compaction). |
| `write_gate` | File-modification boundary; diff review; observer callbacks. |
| `validation_gate` | Post-write structural + semantic validation (`cargo_gate`, `tree_sitter_gate`). |
| `scope_enforcer` | `FileScope` enforcement on agent proposals. |
| `path_pattern` | Glob-aware scope overlap detection (backs DSL scope validation). |
| `session_state` | Checkpointable session state for resume. |
| `repo_map` | Code knowledge graph (tree-sitter) + shallow `topology.rs` for `<repo_topology>` injection. |
| `tree_sitter` | 16-language registry + query loader. |
| `diff_engine` | Unified diff generation/application. |
| `git` | `git2` wrapper, worktrees, branches. |
| `terminal` | PTY + OSC 133 + `vt100` emulation (12-file submodule). |
| `indent` | Hybrid indent detection. |
| `iteration` | Convergence + test-generator loop control. |
| `observer` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` traits. |
| `query_loader` | Tree-sitter query discovery. |
| `types` | Shared boundary types (`FileScope`, `WriteProposal`, `ModelTier`, …). |
| `workspace` | `.gaviero/settings.json` cascade + workspace discovery. |

`tree-sitter` types (`Language`, `Node`, `Parser`, `Query`, `Tree`, …) are re-exported here; downstream crates **must not** depend on the `tree-sitter` crate directly.

## Architecture

**Swarm** ([`swarm/`](src/swarm)) — tier execution (local / cheap / expensive / codex / cursor), parallel fan-out, six-phase pipeline (plan → execute → verify → merge → replan → finalize), dependency DAG, checkpoint/resume, merge-conflict handling, replanner, calibration, context bundle assembly. Backends in [`swarm/backend/`](src/swarm/backend): `claude_code`, `codex`, `cursor`, `ollama`, `mock` — all behind the [`AgentBackend`](src/swarm/backend/mod.rs) trait.

**Memory** ([`memory/`](src/memory)) — SQLite + sqlite-vec, pluggable `Embedder` trait. Default `gte-modernbert-base` (768-dim); `nomic-embed-text-v1.5` is legacy. Multi-DB registry (global / workspace / per-folder) via [`stores.rs`](src/memory/stores.rs). Single-consumer writer task owns all writes ([`writer.rs`](src/memory/writer.rs), `WriterMessage` mpsc) — no callsite holds the SQLite Mutex during embed. Default retrieval is merged multi-scope hybrid (RRF: vector 0.7 + FTS 0.3); legacy narrow→wide cascade with 0.70 early-exit is retained as a kill-switch behind `memory.retrieval.mode = "cascade"`. Optional cross-encoder reranker ([`reranker.rs`](src/memory/reranker.rs)) off by default. Three-cadence consolidation: per-turn [`extractor.rs`](src/memory/extractor.rs) → per-session [`session_consolidator.rs`](src/memory/session_consolidator.rs) → idle/weekly [`sleeptime.rs`](src/memory/sleeptime.rs) + [`sleeptime_scheduler.rs`](src/memory/sleeptime_scheduler.rs). Soft-delete via `/forget` writes a `deletions` audit row; History rows are immutable except via the C2.4 redaction path. Store I/O is split under [`store/`](src/memory/store): `search`, `write`, `panel_ops`, `deletions_ops`, `compression_ops`, `sleeptime_ops`, `telemetry_ops`, `manifest`. The `api-embedders` Cargo feature reserves the hosted-API surface but currently exposes a `NotImplemented` placeholder.

**MCP** ([`mcp/`](src/mcp)) — in-process server exposing three read-only tools to subprocess coding agents: `memory_search`, `blast_radius`, `node_doc` ([`tools.rs`](src/mcp/tools.rs)). Listens on `<workspace>/.gaviero/mcp.sock`; subprocess agents reach it through the `gaviero-mcp-shim` binary. [`config_synth.rs`](src/mcp/config_synth.rs) writes `.mcp.json` (Claude Code), `.codex/config.toml` (Codex), and `cursor.json` per-worktree configs that point at the shim. [`external_memory.rs`](src/mcp/external_memory.rs) detects and disables competing memory MCP servers in agent config with consent. [`observer.rs`](src/mcp/observer.rs) surfaces tool-call events to the host for the TUI audit panel. **Read-only by construction.**

**Agent session** ([`agent_session/`](src/agent_session)) — backend-neutral transport. Codex is dual-mode: [`codex_exec.rs`](src/agent_session/codex_exec.rs) for one-shot exec mode, [`codex_app_server.rs`](src/agent_session/codex_app_server.rs) for persistent app-server mode. [`cursor.rs`](src/agent_session/cursor.rs) wraps the Cursor CLI (native resume mode). [`registry.rs`](src/agent_session/registry.rs) routes by model-spec prefix.

**ACP** ([`acp/`](src/acp)) — legacy direct Claude subprocess path: session factory, argv/tempfile prompt spill (`ARGV_THRESHOLD` in [`session.rs`](src/acp/session.rs)), streaming file-block extraction ([`protocol.rs`](src/acp/protocol.rs)). New code paths use `agent_session::claude` instead; the legacy path remains until parity is reached.

**Repo map** ([`repo_map/`](src/repo_map)) — `builder.rs` walks the tree-sitter graph; `page_rank.rs` ranks nodes; `topology.rs` produces the shallow filesystem-only folder tree injected as `<repo_topology>` on every first turn. Together they back the two-layer graph context (`<repo_topology>` + `<repo_outline>`).

**Write Gate** ([`write_gate.rs`](src/write_gate.rs)) — modes: `Interactive` (queue → TUI review), `AutoAccept` (validate + write), `Deferred` (batch), `RejectAll` (drop silently). Diff review + scope validation + observer callbacks happen here.

## Conventions

- Lock discipline: never hold `Mutex` across I/O, parsing, or embedding. The memory writer task is the **single** owner of SQLite writes.
- `AgentBackend` trait is object-safe; every backend in [`swarm/backend/`](src/swarm/backend) implements it.
- Embedder is pluggable (`Embedder` trait + `model_manager::resolve_embedder_model`); default `gte-modernbert-base` (768-dim). Cosine similarity.
- Memory writes require explicit `WriteScope` — never infer. All writes flow through the writer task.
- Scoring (see [`memory/scoring.rs`](src/memory/scoring.rs)): 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights. B4 recency floor + decay-exempt types (`Decision` / `Convention` / `Invariant` / `Preference`) protect reference memories from age-out.
- Model spec canonical form is `provider:model`. `validate_model_spec` ([`swarm/backend/shared.rs`](src/swarm/backend/shared.rs)) rejects bare names.
- Tree-sitter access goes through `gaviero_core::{Language, Parser, Query, …}` re-exports; never `use tree_sitter::*` in downstream crates.

## Rules

- **MCP tools are read-only.** Never add a write tool to [`mcp/tools.rs`](src/mcp/tools.rs); route writes through the Write Gate or the memory writer task.
- **No UI deps.** This crate compiles without `ratatui`, `crossterm`, or any TUI crate. `tui-term`/`vt100` are allowed because the embedded terminal subsystem lives here.
- **No DSL deps.** This crate must not depend on `gaviero-dsl`; the dependency arrow points the other way.
- **History rows are immutable** except via the C2.4 redaction path (`forget_history` requires explicit literal-string confirm + reason).
- **Decay-exempt types** must not be aged out by sleeptime; check the kind set before pruning.

## Dependencies

- `tree-sitter 0.25` + 16 grammars (Java, JS, TS, Rust, HTML, CSS, JSON, Bash, TOML, C, C++, LaTeX, Python, YAML, Kotlin, `tree-sitter-gaviero`).
- `git2 0.19` — worktrees, branches, diff.
- `rusqlite 0.32` (bundled) + `sqlite-vec 0.1.8` — memory store.
- `ort 2.0.0-rc.12` + `tokenizers 0.19` + `ndarray 0.17` — ONNX inference.
- `petgraph 0.8` — DAG ops in the swarm planner.
- `portable-pty 0.9` + `vt100 0.16` — terminal emulation.
- `rmcp 1.5` + `schemars 1.2` — in-process MCP server.
- `zstd 0.13` + `bincode 1.3` — History compression (sleeptime, ≥90 days).
- `reqwest 0.12`, `async-trait`, `futures`, `tokio-stream`, `tokio-util`, `chrono`, `toml`, `base64`, `sha2`, `tempfile`, `ropey`, `similar`, `streaming-iterator`.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — module map, swarm/memory pipelines, MCP topology, write-gate flow.
- [README.md](README.md) — public-API reference.
- [../../ARCHITECTURE.md](../../ARCHITECTURE.md) — workspace-wide design.
