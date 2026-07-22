# gaviero-mcp-shim

## Overview

A tiny stdio↔socket bridge that connects subprocess coding agents (Claude Code, Codex, Cursor) to Gaviero's in-process MCP server. The agent speaks MCP over stdio; the shim forwards bytes to Gaviero's per-workspace endpoint — Unix socket `<workspace>/.gaviero/mcp.sock` or Windows named pipe `\\.\pipe\gaviero-<hash>`.

Zero workspace dependencies (`tokio`, `clap`, `anyhow`, `tracing` only). Gaviero writes per-worktree agent configs automatically via `gaviero_core::mcp::config_synth`.

## Installation

```bash
cargo build -p gaviero-mcp-shim --release
# Binary: target/release/gaviero-mcp-shim

cargo install --path crates/gaviero-mcp-shim   # optional: put on PATH
```

Subprocess agents need the shim on `PATH` or referenced by absolute path in their MCP config `command` field.

## Usage

```bash
# Unix
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock

# Windows
gaviero-mcp-shim --pipe '\\.\pipe\gaviero-<hash>'

# Widen initial connect retry window
gaviero-mcp-shim --socket /path/to/.gaviero/mcp.sock --connect-timeout-secs 10
```

MCP requests on stdin, responses on stdout; diagnostics on stderr (WARN level).

## Examples

Typical agent MCP config (written automatically by Gaviero):

```json
{
  "mcpServers": {
    "gaviero": {
      "command": "gaviero-mcp-shim",
      "args": ["--socket", "/path/to/.gaviero/mcp.sock"]
    }
  }
}
```

The shim retries connection with exponential backoff (50 ms → 400 ms) if Gaviero is still starting.

## Configuration

| Flag | Default | Description |
|---|---|---|
| `--socket <path>` | required on Unix | Workspace MCP Unix socket path |
| `--pipe <name>` | required on Windows | Windows named-pipe name |
| `--connect-timeout-secs <n>` | `5` | Total seconds for initial connect retries |

## API

No library API — single binary with a `bridge` loop using byte-faithful `tokio::io::copy` in both directions. No MCP parsing or access control; that lives in `gaviero-core::mcp`.

**Tools exposed by the server** (read-only; shim is unaware):

| Tool | Availability |
|---|---|
| `memory_search`, `memory_get` | Always |
| `blast_radius`, `node_doc`, `repo_outline` | Always |
| `symbol_search`, `symbol_doc` | `repoMap.symbolEnrichment.enabled` |

## See Also

- [gaviero-core](../gaviero-core/README.md) — MCP server and tool definitions
- [Root README](../../README.md) — overall architecture

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
