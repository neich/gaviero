# gaviero-mcp-shim

A tiny stdio↔socket bridge that connects subprocess coding agents (Claude Code, Codex, Cursor) to Gaviero's in-process MCP server.

## Overview

When Gaviero spawns a subprocess coding agent, that agent expects to reach an MCP server over stdio. `gaviero-mcp-shim` is the binary it launches: it opens a connection to Gaviero's per-workspace MCP endpoint — the Unix socket `<workspace>/.gaviero/mcp.sock` on Unix, or a `\\.\pipe\gaviero-<hash>` named pipe on Windows — and copies bytes in both directions. Gaviero's in-process `rmcp` server on the other end speaks the actual MCP protocol.

The shim has **zero workspace dependencies** — only `tokio`, `clap`, `anyhow`, and `tracing`/`tracing-subscriber`. It builds and installs independently of the rest of the workspace, which keeps the binary small (a few KB) so an MCP config's `command` field resolves cleanly everywhere.

Why a separate process:

- Subprocess agents never need to know Gaviero's internals — they just speak MCP over stdio.
- Gaviero can restart (re-running `Workspace::open`) without forcing the agent to restart: the shim retries the connect with exponential backoff (50 ms → 400 ms).
- It is a dumb, byte-faithful pipe — no MCP parsing, framing, or access-control logic lives here.

## Installation

```bash
cargo build -p gaviero-mcp-shim --release
# Binary: target/release/gaviero-mcp-shim

# Optional: put it on PATH so agent configs can reference it by name
cargo install --path crates/gaviero-mcp-shim
```

Subprocess agents locate the shim in one of two ways:

- Install it on `PATH`, or
- Reference it by absolute path in the agent's MCP config (`command` field).

Gaviero writes the per-worktree agent configs automatically via `gaviero_core::mcp::config_synth` (`.mcp.json` for Claude Code, `.codex/config.toml` for Codex, `.cursor/mcp.json` for Cursor), so in normal use you never invoke the shim by hand.

## Usage

```bash
# Unix — connect to the workspace socket
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock

# Windows — connect to the workspace named pipe
gaviero-mcp-shim --pipe '\\.\pipe\gaviero-<hash>'

# Widen the initial connect retry window (e.g. slow workspace open)
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock --connect-timeout-secs 10
```

The shim reads MCP requests on stdin and writes responses to stdout; all diagnostics go to stderr (`tracing-subscriber` at WARN level).

## Configuration

| Flag | Default | Description |
|---|---|---|
| `--socket <path>` | required on Unix | Absolute path to the workspace MCP Unix socket (`<workspace>/.gaviero/mcp.sock`). Unix only. |
| `--pipe <name>` | required on Windows | Windows named-pipe name (`\\.\pipe\gaviero-…`). Windows only. |
| `--connect-timeout-secs <n>` | `5` | Total seconds the initial connect will retry before failing. Useful when the agent spawns before Gaviero finishes `Workspace::open`. |

## Protocol

MCP over stdio is line-delimited JSON-RPC 2.0. The shim uses byte-faithful copy loops (`tokio::io::copy`) in both directions and never parses or reframes traffic — `rmcp` on the server side depends on byte-faithful delivery. The bridge exits cleanly when either side closes.

## MCP tools exposed

The tools live on the server side (`gaviero-core/src/mcp/`), not in the shim. The in-process server exposes **read-only tools only** — no backend can write to disk through MCP; every file change goes through Gaviero's Write Gate.

| Tool | Description | Availability |
|---|---|---|
| `memory_search` | Semantic search over the workspace memory store | Always |
| `memory_get` | Fetch a specific memory row by id | Always |
| `blast_radius` | Graph-based impact analysis for a set of files | Always |
| `node_doc` | Documentation/summary for a named code symbol | Always |
| `repo_outline` | PageRank-ranked outline of the repository | Always |
| `symbol_search` | Semantic search over enriched symbol docs | `repoMap.symbolEnrichment.enabled` |
| `symbol_doc` | Full doc for a resolved symbol | `repoMap.symbolEnrichment.enabled` |

The shim has no MCP awareness of its own — tool semantics, schemas, and access control live entirely in `gaviero-core::mcp`. Keep it a dumb pipe.

## See Also

- [`crates/gaviero-core/README.md`](../gaviero-core/README.md) — MCP server implementation and tool definitions
- [Root README](../../README.md) — overall architecture and features

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
