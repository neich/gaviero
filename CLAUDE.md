# Gaviero

Terminal editor + headless CLI. AI agent orchestration. Rust 2024.

## Build & Test

```bash
cargo build                    # all crates
cargo test                     # all tests
cargo test -p gaviero-core     # core only
cargo test -p gaviero-dsl      # DSL only
cargo clippy --workspace       # lint
```

Binaries: `gaviero` (TUI), `gaviero-cli` (headless).

## Workspace

```
crates/
  gaviero-core/        Core: swarm, memory, ACP, write gate, git, indent, terminal
  gaviero-tui/         TUI binary (ratatui + crossterm)
  gaviero-cli/         Headless runner (clap)
  gaviero-dsl/         .gaviero compiler (logos + chumsky -> CompiledPlan)
  tree-sitter-gaviero/ Tree-sitter grammar for .gaviero files
```

Pipeline logic -> `gaviero-core`. TUI = render + input. CLI = args + observers. Core has no UI dep.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md).

## Conventions

- `anyhow::Result` everywhere. `thiserror` for DSL domain errors.
- Tokio runtime. Never hold Mutex across I/O or CPU work.
- `tracing` for logging: `debug!`/`info!`/`warn!`/`error!`.
- `serde` derive on all boundary types.
- Tree-sitter re-exports from `gaviero-core::lib.rs`. Never depend `tree-sitter` directly downstream.
- Agent writes flow through Write Gate. Agents get read-only tools (`Read`, `Glob`, `Grep`).
- `git2` only. Never shell out to `git`.
- Rust 2024 edition.

## Rules

- Never bypass scope validation. Agents stay within `FileScope`.
- Never hold WriteGatePipeline Mutex across diff, tree-sitter, or disk I/O.
- Embedding runs outside SQLite Mutex. Lock protects DB I/O only.
- Swarm branches: `gaviero/{work_unit_id}`.
- Worktrees: `.gaviero/worktrees/{id}/`, cleanup via `Drop`.
- Memory writes require explicit `WriteScope`. Never guess scope.
- Scoped search cascades narrow->wide, early-terminates at confidence threshold.

## MCP Servers

- Use context7 MCP for library docs.
- MCP memory: namespace `Gaviero`. Store memories AI-optimized, not human-readable.
