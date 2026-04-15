# Gaviero

Terminal editor + headless CLI. AI agent orchestration. Rust 2024.

## Build & Test

```bash
cargo build                    # all crates
cargo test                     # all tests
cargo clippy --workspace       # lint
```

Binaries: `gaviero` (TUI), `gaviero-cli` (headless).

## Workspace

Five crates — see per-crate CLAUDE.md for details:
- `gaviero-core/` — all pipeline logic (no UI deps)
- `gaviero-tui/` — terminal UI (ratatui + crossterm)
- `gaviero-cli/` — headless runner (clap)
- `gaviero-dsl/` — `.gaviero` compiler (logos + chumsky)
- `tree-sitter-gaviero/` — tree-sitter grammar

Architecture: pipeline logic in core. TUI/CLI are thin wrappers with observers.

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
- Never hold WriteGatePipeline Mutex across diff, tree-sitter, or disk I/O.
- Embedding runs outside SQLite Mutex. Lock protects DB I/O only.
- Swarm branches: `gaviero/{work_unit_id}`.
- Worktrees: `.gaviero/worktrees/{id}/`, cleanup via `Drop`.
- Memory writes require explicit `WriteScope`. Never infer scope.
- Scoped search cascades narrow→wide, early-terminates at 0.70 confidence.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — system design, data flow, subsystems
- [.gaviero/docs-inventory.md](.gaviero/docs-inventory.md) — documentation status, gaps, TODOs
