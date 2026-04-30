# Gaviero

Terminal editor + headless CLI. AI agent orchestration. Rust 2024.

## Build & Test

```bash
cargo build                    # all crates
cargo test                     # all tests
cargo clippy --workspace       # lint
```

Binaries: `gaviero` (TUI), `gaviero-cli` (headless), `gaviero-mcp-shim` (subprocess→MCP bridge).

## Workspace

Six crates — see per-crate CLAUDE.md for details:
- [`gaviero-core/`](crates/gaviero-core/CLAUDE.md) — all runtime logic; no UI/DSL deps
- [`gaviero-tui/`](crates/gaviero-tui/CLAUDE.md) — terminal UI (ratatui + crossterm)
- [`gaviero-cli/`](crates/gaviero-cli/CLAUDE.md) — headless runner (clap)
- [`gaviero-dsl/`](crates/gaviero-dsl/CLAUDE.md) — `.gaviero` compiler (logos + chumsky)
- `gaviero-mcp-shim/` — stdio↔Unix-socket bridge for subprocess agents to reach core's in-process MCP server (read-only memory/graph tools). Self-contained binary; no workspace deps.
- [`tree-sitter-gaviero/`](crates/tree-sitter-gaviero/CLAUDE.md) — tree-sitter grammar

Architecture: pipeline logic in core. TUI/CLI are thin wrappers with observers. DSL depends on core. Subprocess agents (Claude Code, Codex) reach core's MCP tools via the shim over `<workspace>/.gaviero/mcp.sock`.

## Conventions

- `anyhow::Result` everywhere. `thiserror` for DSL domain errors.
- Tokio runtime. Never hold Mutex across I/O or CPU work.
- `tracing` for logging: `debug!`/`info!`/`warn!`/`error!`.
- `serde` derive on all boundary types.
- Tree-sitter re-exports from `gaviero-core::lib.rs`. Never import `tree-sitter` downstream.
- Agent writes flow through Write Gate. Agents get read-only tools.
- `git2` only. Never shell out to `git`.

## Rules

- Never bypass scope validation. Agents stay within `FileScope`.
- Scope overlap is enforced via `gaviero-core::path_pattern` — disjoint glob patterns are allowed; literal-prefix overlap is not.
- Never hold WriteGatePipeline Mutex across diff, tree-sitter, or disk I/O.
- Embedding runs outside SQLite Mutex. Lock protects DB I/O only. All memory writes flow through the single-consumer writer task (`WriterMessage` mpsc).
- MCP server (`gaviero-core::mcp`) is read-only by construction. Never add a write tool there; writes go through the Write Gate or the memory writer task.
- Swarm branches: `gaviero/{work_unit_id}`.
- Worktrees: `.gaviero/worktrees/{id}/`, cleanup via `Drop`.
- Memory writes require explicit `WriteScope`. Never infer scope.
- Default retrieval is merged multi-scope hybrid (RRF: vector 0.7 + FTS 0.3). The legacy narrow→wide cascade with 0.70 early-exit is a kill-switch behind `memory.retrieval.mode = "cascade"`; do not assume it's the active path.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — system design, data flow, subsystems
- [.gaviero/docs-inventory.md](.gaviero/docs-inventory.md) — documentation status, gaps, TODOs
