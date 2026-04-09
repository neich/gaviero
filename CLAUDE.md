# Gaviero

Terminal code editor + headless CLI for AI agent orchestration, written in Rust 2024.

## Build & Test

```bash
cargo build                    # all crates
cargo test                     # all tests
cargo test -p gaviero-core     # core only
cargo test -p gaviero-dsl      # DSL only
cargo clippy --workspace       # lint
```

Binaries: `gaviero` (TUI), `gaviero-cli` (headless runner).

## Workspace Layout

```
crates/
  gaviero-core/       Core logic: swarm, memory, ACP, write gate, git, indent, terminal
  gaviero-tui/        TUI editor binary (ratatui + crossterm)
  gaviero-cli/        Headless CLI runner (clap)
  gaviero-dsl/        .gaviero script compiler (logos + chumsky → CompiledPlan)
  tree-sitter-gaviero/ Tree-sitter grammar for .gaviero files
```

**Separation rule:** All pipeline logic lives in `gaviero-core`. TUI = rendering + input only. CLI = arg parsing + observers only. Core has no UI dependency.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for data flows, abstractions, and module maps.

## Conventions

- **Error handling:** Use `anyhow::Result` for fallible functions. Use `thiserror` for domain error types in DSL.
- **Async:** Tokio runtime everywhere. Never hold a Mutex across I/O or CPU-heavy work.
- **Logging:** `tracing` crate. Use `tracing::debug!` / `info!` / `warn!` / `error!`.
- **Serialization:** `serde` with derive for all data types crossing boundaries.
- **Tree-sitter:** Always use re-exports from `gaviero-core::lib.rs`. Never depend on `tree-sitter` directly from downstream crates.
- **Agent writes:** All file mutations flow through the Write Gate pipeline. Agents get read-only tools only (`Read`, `Glob`, `Grep`).
- **Git operations:** Use `git2` crate, never shell out to `git` CLI.
- **Edition:** Rust 2024 for all crates.

## Key Rules

- Never bypass scope validation — agents must not write outside their `FileScope`.
- Never hold the WriteGatePipeline Mutex across diff computation, tree-sitter parsing, or disk I/O.
- Embedding computation runs outside the SQLite Mutex — lock protects only DB I/O.
- Swarm branches follow the pattern `gaviero/{work_unit_id}`.
- Worktrees live in `.gaviero/worktrees/{id}/` and are cleaned up via `Drop`.
- Memory writes always require an explicit `WriteScope` — the system never guesses scope level.
- Scoped memory search cascades narrowest-to-widest with early termination at confidence threshold.

## MCP Servers

- Always use context7 MCP server to search for documentation.
- Use MCP memory server with namespace `Gaviero` to store and retrieve project memories. Memories must be stored tailored for AI to maximize information and determinism, not for human reading.
