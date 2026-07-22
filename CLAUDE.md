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
- [`gaviero-mcp-shim/`](crates/gaviero-mcp-shim/CLAUDE.md) — stdio↔socket bridge (Unix socket / Windows named pipe). Zero workspace deps.
- [`tree-sitter-gaviero/`](crates/tree-sitter-gaviero/CLAUDE.md) — `.gaviero` grammar.

Dependency rules: core has no UI/DSL deps. `tui` and `cli` depend on `core` + `dsl`. `dsl` depends on `core`. `gaviero-mcp-shim` is self-contained and reaches core only over the workspace MCP endpoint (`McpEndpoint`: `<workspace>/.gaviero/mcp.sock` on Unix, `\\.\pipe\gaviero-<hash>` on Windows). See [ARCHITECTURE.md](ARCHITECTURE.md) for the full topology.

## Architecture

Pipeline logic lives in `gaviero-core`. The TUI and CLI are thin wrappers that wire observers (`WriteGateObserver`, `AcpObserver`, `SwarmObserver` — [crates/gaviero-core/src/observer.rs](crates/gaviero-core/src/observer.rs)) to surface agent activity.

Subprocess coding agents (Claude Code, Codex, Cursor) reach core's in-process MCP server (read-only memory + graph tools) by spawning `gaviero-mcp-shim`, which pipes stdio to the workspace MCP endpoint (Unix socket / Windows named pipe — [crates/gaviero-core/src/mcp/transport.rs](crates/gaviero-core/src/mcp/transport.rs)). DeepSeek (`deepseek:`) runs in-process via [`tool_agent`](crates/gaviero-core/src/agent_session/tool_agent) + [`DeepseekBackend`](crates/gaviero-core/src/swarm/backend/deepseek.rs); it does not use the shim.

`.gaviero-workspace` files (any basename, fixed extension) describe multi-folder workspaces; bare directories are treated as single-folder workspaces. Dispatched at TUI startup in [crates/gaviero-tui/src/main.rs](crates/gaviero-tui/src/main.rs).

Tier overrides for DSL scripts live in `examples/profiles/*.gaviero` (`doc-claude`, `doc-codex`, `doc-cursor`) and are loaded via `gaviero-cli --tiers-file <path>` (`tier <alias> <client>` lines only — [crates/gaviero-dsl/src/tiers.rs](crates/gaviero-dsl/src/tiers.rs)).

### Agent Runtime Parity

All interactive coding providers — Claude Code, Codex, Cursor, Ollama, DeepSeek — must expose the same user-facing contract:

- **Observable while running.** Reasoning deltas, tool starts, streaming status, file-proposal summaries, completion, and token usage flow through `AcpObserver` (or the swarm `UnifiedStreamEvent` adapter — [crates/gaviero-core/src/swarm/backend/mod.rs](crates/gaviero-core/src/swarm/backend/mod.rs)).
- **File edits never bypass review.** Native edit-capable providers (Claude Code, Codex, Cursor) route writes through their tool-call channel; the host turns each into a `WriteProposal` and runs it through the Write Gate. Stream-only providers (Ollama) and in-process tool agents (DeepSeek Option-B) emit complete `<file path="relative/path">…</file>` blocks; the host extracts them via [crates/gaviero-core/src/acp/protocol.rs](crates/gaviero-core/src/acp/protocol.rs) and routes them through the same Write Gate.
- **Single Write Gate.** Every file change passes through `write_gate::WriteGatePipeline` ([crates/gaviero-core/src/write_gate.rs](crates/gaviero-core/src/write_gate.rs)). No backend writes to disk directly.
- **Scope enforcement.** Proposals are checked against the active `FileScope` ([crates/gaviero-core/src/scope_enforcer.rs](crates/gaviero-core/src/scope_enforcer.rs)) before they leave the gate.
- **MCP is read-only.** Seven tools: `memory_search`, `memory_get`, `blast_radius`, `node_doc`, `repo_outline`, plus `symbol_search` / `symbol_doc` behind `repoMap.symbolEnrichment.enabled` ([crates/gaviero-core/src/mcp/tools.rs](crates/gaviero-core/src/mcp/tools.rs)). Never add a write tool; route writes through the Write Gate or the memory writer task.

## Conventions

- **Model spec is `provider:model`.** Bare names are rejected at dispatch (`validate_model_spec`, [crates/gaviero-core/src/swarm/backend/shared.rs](crates/gaviero-core/src/swarm/backend/shared.rs)). Prefixes: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:`.
- **Lock discipline.** Never hold a `Mutex` across I/O, parsing, or embedding. The memory `writer` task is the single owner of SQLite writes.
- **Two-layer graph context.** The pre-prompt assembler injects `<repo_topology>` (shallow filesystem-only folder map, [crates/gaviero-core/src/repo_map/topology.rs](crates/gaviero-core/src/repo_map/topology.rs)) plus `<repo_outline>` (PageRank-ranked code outline). The TUI `/lite` chat command drops `<repo_outline>` + memory + impact and keeps only topology.
- **Plan production.** When drafting implementation plans for other agents, assume Claude Code (`claude:fable` / `claude:opus`) or Codex (`codex:gpt-5.5`) unless the user will implement themselves. Plans must be agent-executable: concrete work units, ownership boundaries, expected files/modules, verification steps, sequencing constraints. Example client roster: [crates/gaviero-dsl/examples/clients.gaviero](crates/gaviero-dsl/examples/clients.gaviero).

## Rules

- Never bypass the Write Gate. Every file change is a `WriteProposal`.
- Never add write tools to `mcp/`. MCP is read-only by construction.
- Never hold a `Mutex` across `.await`, embeddings, or filesystem I/O.
- Never emit a bare model name; always `provider:model`.
- Never edit `tree-sitter-gaviero/src/parser.c` or `grammar.json` by hand — regenerate from `grammar.js`.
- There is no `--no-memory` CLI flag; do not document or invent one — check [`Cli`](crates/gaviero-cli/src/main.rs) before adding flag docs.

## Dependencies

Shared versions live in [`[workspace.dependencies]`](Cargo.toml) (tokio, serde, clap, reqwest, logos, chumsky, miette, …) — inherit with `{ workspace = true }`; add a crate-local version only for crate-specific deps. `tree-sitter 0.25` enters the graph exactly once, through `gaviero-core`'s re-exports ([crates/gaviero-core/src/lib.rs](crates/gaviero-core/src/lib.rs)) — never depend on it directly. Per-crate dependency lists live in each crate's CLAUDE.md.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — workspace-wide design, six-phase swarm pipeline, memory pipeline, MCP topology.
- [README.md](README.md) — user-facing feature reference.
