# gaviero-mcp-shim

A tiny stdio↔socket bridge that connects subprocess coding agents (Claude Code, Codex, Cursor) to Gaviero's in-process MCP server.

## Overview

When Gaviero spawns a subprocess agent, the agent expects to talk to an MCP server over stdio. `gaviero-mcp-shim` is the binary it spawns: it opens a connection to Gaviero's workspace endpoint — the Unix socket `<workspace>/.gaviero/mcp.sock` on Unix, a `\\.\pipe\gaviero-<hash>` named pipe on Windows — and copies bytes in both directions. Gaviero's in-process `rmcp` server on the other end handles the actual MCP protocol.

**No workspace dependencies.** This crate uses only `tokio`, `clap`, `anyhow`, and `tracing`. It builds and installs independently of the rest of the workspace.

Benefits of this design:
- Subprocess agents don't need to know Gaviero's internals
- Gaviero can restart without requiring the subprocess to restart — the shim retries the socket connect with exponential backoff
- The binary is a few KB, so `.mcp.json`'s `command` field resolves cleanly everywhere

## Installation & Build

```bash
cargo build -p gaviero-mcp-shim --release
# Binary: target/release/gaviero-mcp-shim
```

For subprocess agents to find the shim, either:
- Copy `gaviero-mcp-shim` to a directory on `PATH`, or
- Use an absolute path in the agent's MCP config (`command` field)

## Usage

```bash
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock            # Unix
gaviero-mcp-shim --pipe '\\.\pipe\gaviero-<hash>'               # Windows
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock --connect-timeout-secs 10
```

Gaviero writes the per-workspace MCP config automatically (via `mcp::config_synth`), so you typically never run this manually.

## Flags

| Flag | Default | Description |
|---|---|---|
| `--socket <path>` | — | Absolute path to the workspace MCP Unix socket (`<workspace>/.gaviero/mcp.sock`). Unix only. |
| `--pipe <name>` | — | Windows named-pipe name (`\\.\pipe\gaviero-…`). Windows only. |
| `--connect-timeout-secs <n>` | `5` | Retry window for the initial connect. Useful when the agent spawns before Gaviero finishes `Workspace::open`. |

## Protocol

MCP over stdio is line-delimited JSON-RPC 2.0. The shim uses `tokio::io::copy` — byte-faithful, no framing — which is exactly what `rmcp` expects.

## MCP Tools Exposed

The server on the other end (`gaviero-core/src/mcp/`) exposes three **read-only** tools:

| Tool | Description |
|---|---|
| `memory_search` | Semantic search over the workspace memory store |
| `blast_radius` | Graph-based impact analysis for a set of files |
| `node_doc` | Documentation/summary for a named code symbol |

No write tools are exposed. Writes always go through Gaviero's Write Gate.

## See Also

- [`crates/gaviero-core/src/mcp/`](../gaviero-core/README.md) — server implementation and tool definitions
- [Root README](../../README.md) — overall architecture
