# Gaviero

Terminal editor + headless CLI for AI agent orchestration. Rust 2024 workspace.

## Build & Test

```bash
cargo build                    # all crates
cargo test                     # all tests (network/model tests are #[ignore])
cargo clippy --workspace       # lint
```

Binaries: `gaviero` (TUI), `gaviero-cli` (headless), `gaviero-mcp-shim` (subprocess→MCP bridge).

## Workspace

Six crates — read the per-crate `CLAUDE.md` before touching its source.

- [`gaviero-core/`](crates/gaviero-core/CLAUDE.md) — all runtime logic; no UI/DSL deps.
- [`gaviero-tui/`](crates/gaviero-tui/CLAUDE.md) — terminal UI (ratatui + crossterm).
- [`gaviero-cli/`](crates/gaviero-cli/CLAUDE.md) — headless runner (clap).
- [`gaviero-dsl/`](crates/gaviero-dsl/CLAUDE.md) — `.gaviero` workflow compiler (logos + chumsky).
- [`gaviero-mcp-shim/`](crates/gaviero-mcp-shim/CLAUDE.md) — stdio↔Unix-socket bridge. Zero workspace deps.
- [`tree-sitter-gaviero/`](crates/tree-sitter-gaviero/CLAUDE.md) — `.gaviero` grammar.

Dependency rules: core has no UI/DSL deps. `tui` and `cli` depend on `core` + `dsl`. `dsl` depends on `core`. `gaviero-mcp-shim` is self-contained and reaches core only over `<workspace>/.gaviero/mcp.sock`. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full topology.

## Architecture (orientation)

Pipeline logic lives in `gaviero-core`. The TUI and CLI are thin wrappers that wire observers (`WriteGateObserver`, `AcpObserver`, `SwarmObserver` — [crates/gaviero-core/src/observer.rs](crates/gaviero-core/src/observer.rs)) to surface agent activity.

Subprocess coding agents (Claude Code, Codex, Cursor) reach core's in-process MCP server (read-only memory + graph tools) by spawning `gaviero-mcp-shim`, which pipes stdio to `<workspace>/.gaviero/mcp.sock`.

`.gaviero-workspace` files (any basename, fixed extension) describe multi-folder workspaces; bare directories are treated as single-folder workspaces. Dispatched at TUI startup in [crates/gaviero-tui/src/main.rs](crates/gaviero-tui/src/main.rs).

Tier overrides for DSL scripts live in `examples/profiles/*.gaviero` files and are loaded via `gaviero-cli --tiers-file <path>` (`tier <alias> <client>` lines only — see [crates/gaviero-dsl/src/tiers.rs](crates/gaviero-dsl/src/tiers.rs)).

## Agent Runtime Parity

All interactive coding providers — Claude Code, Codex, Cursor, Ollama — must expose the same user-facing contract:

- **Observable while running.** Reasoning deltas, tool starts, streaming status, file-proposal summaries, completion, and token usage flow through `AcpObserver` (or the swarm `UnifiedStreamEvent` adapter — [crates/gaviero-core/src/swarm/backend/mod.rs](crates/gaviero-core/src/swarm/backend/mod.rs)).
- **File edits never bypass review.** Native edit-capable providers (Claude Code, Codex, Cursor) route writes through their own tool-call channel; the host turns each tool call into a `WriteProposal` and runs it through the Write Gate. Stream-only providers (Ollama) emit complete `<file path="relative/path">…</file>` blocks inline; the swarm executor extracts them via [crates/gaviero-core/src/acp/protocol.rs](crates/gaviero-core/src/acp/protocol.rs) and routes them through the same Write Gate.
- **Single Write Gate.** Every file change — proposed, modified, or deleted — passes through `write_gate::WriteGatePipeline` ([crates/gaviero-core/src/write_gate.rs](crates/gaviero-core/src/write_gate.rs)). No backend writes to disk directly.
- **Scope enforcement.** Proposals are checked against the active `FileScope` ([crates/gaviero-core/src/scope_enforcer.rs](crates/gaviero-core/src/scope_enforcer.rs)) before they leave the gate.
- **MCP is read-only.** The in-process MCP server exposes `memory_search`, `blast_radius`, `node_doc` only ([crates/gaviero-core/src/mcp/tools.rs](crates/gaviero-core/src/mcp/tools.rs)). Never add a write tool there; route writes through the Write Gate or the memory writer task.

## Conventions

- **Model spec is `provider:model`.** Bare names are rejected at dispatch (`validate_model_spec`, [crates/gaviero-core/src/swarm/backend/shared.rs](crates/gaviero-core/src/swarm/backend/shared.rs)). Prefixes: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`.
- **Lock discipline.** Never hold a `Mutex` across I/O, parsing, or embedding. The memory `writer` task is the single owner of SQLite writes.
- **Two-layer graph context.** The pre-prompt assembler injects `<repo_topology>` (shallow filesystem-only folder map, [crates/gaviero-core/src/repo_map/topology.rs](crates/gaviero-core/src/repo_map/topology.rs)) plus `<repo_outline>` (PageRank-ranked code outline). The TUI `/lite` chat command drops `<repo_outline>` + memory + impact and keeps only topology.

## Plan Production

When drafting implementation plans, assume the implementors are Claude Code Opus 4.7 or Codex 5.5 unless the user explicitly says they will implement the work themselves. Plans should be written for agent execution by default: concrete work units, ownership boundaries, expected files/modules, verification steps, sequencing constraints.

## Rules

- Never bypass the Write Gate. Every file change is a `WriteProposal`.
- Never add write tools to `mcp/`. MCP is read-only by construction.
- Never hold a `Mutex` across `.await`, embeddings, or filesystem I/O.
- Never emit a bare model name; always `provider:model`.
- Never edit `tree-sitter-gaviero/src/parser.c` or `grammar.json` by hand — regenerate from `grammar.js`.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — workspace-wide design, six-phase swarm pipeline, memory pipeline, MCP topology.
- [README.md](README.md) — user-facing feature reference.
