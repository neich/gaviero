# Gaviero

Terminal editor + headless CLI. AI agent orchestration. Rust 2024.

## Plan Production

When drafting implementation plans, assume the implementors are Claude Code Opus 4.7 or Codex 5.5 unless the user explicitly says they will implement the work themselves.

Plans should be written for agent execution by default: concrete work units, ownership boundaries, expected files/modules, verification steps, and any sequencing constraints.

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

## Agent Runtime Parity

All interactive coding providers should expose the same user-facing contract:

- Agent activity is observable while it runs. Reasoning deltas, tool starts, streaming status, file proposal summaries, completion, and token usage must flow through `AcpObserver` or an equivalent adapter.
- File edits never bypass review. Native edit-capable providers use their tool-call channel; text-only or stream-only providers emit complete `<file path="relative/path">...