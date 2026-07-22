# gaviero-mcp-shim

Stdio↔endpoint bridge (~187 lines). Subprocess coding agents (Claude Code, Codex, Cursor) spawn this binary as their MCP "server"; it connects to Gaviero's workspace endpoint — Unix domain socket or Windows named pipe — and pipes bytes both ways. The MCP protocol terminates at the in-process rmcp server in [`gaviero-core`](../gaviero-core/CLAUDE.md).

Binary: `gaviero-mcp-shim` ([src/main.rs](src/main.rs)).

## Build & Test

```bash
cargo build -p gaviero-mcp-shim --release
cargo install --path crates/gaviero-mcp-shim   # put on PATH for subprocess agents
```

Per-worktree configs (`.mcp.json`, `.codex/config.toml`, `.cursor/mcp.json`) reference the shim by name — install on `PATH` or use an absolute path. Written by [`gaviero_core::mcp::config_synth`](../gaviero-core/src/mcp/config_synth.rs).

## Architecture

- `connect_with_backoff` (`unix` / `windows` modules) — exponential backoff (50 ms → 400 ms) until deadline; Windows folds `ERROR_PIPE_BUSY` into the same loop.
- `bridge` — bidirectional `tokio::select!` stdin↔endpoint↔stdout; byte-faithful; exits when either side closes.
- Does **not** parse JSON-RPC — `rmcp` on the server expects intact framing.

Endpoint shape: `<workspace>/.gaviero/mcp.sock` or `\\.\pipe\gaviero-<hash>` ([`gaviero_core::mcp::transport`](../gaviero-core/src/mcp/transport.rs)).

### Flags

| Flag | Default | Purpose |
|---|---|---|
| `--socket <path>` | required on Unix | Absolute path to `mcp.sock`. |
| `--pipe <name>` | required on Windows | Named-pipe name (`\\.\pipe\gaviero-…`). |
| `--connect-timeout-secs <N>` | `5` | Connect retry budget. |

`tracing-subscriber` at WARN on stderr.

## Conventions

- **Zero workspace dependencies.** Links only `tokio`, `clap`, `anyhow`, `tracing`, `tracing-subscriber`. No `gaviero-core` / `gaviero-dsl`.
- **Byte-faithful piping.** Never parse, log, or transform MCP traffic.
- **Stderr-only logging.** Stdout is reserved for MCP responses.

## Rules

- **Do not pull in workspace deps.** Features that need core belong on the server side.
- **Do not write to stdout** outside the socket→stdout loop. Diagnostics go through `tracing` on stderr.
- **Connect retries are bounded.** Past the deadline, return the `io::Error` with `Context` — never retry forever.
- **No MCP awareness.** Tool semantics live in [`gaviero_core::mcp`](../gaviero-core/src/mcp). Keep the shim a dumb pipe.

## Dependencies

- `tokio` (full) — async runtime, `UnixStream` / named pipe, stdio.
- `clap` (derive) — `--socket`, `--pipe`, `--connect-timeout-secs`.
- `anyhow` — connect/copy error context.
- `tracing` + `tracing-subscriber` — stderr WARN logger.

## See Also

- [`gaviero_core::mcp`](../gaviero-core/src/mcp) — tools, config synth, observer, external-memory detection.
- [ARCHITECTURE.md](ARCHITECTURE.md) — bridge topology.
- [`../../CLAUDE.md`](../../CLAUDE.md) — MCP read-only invariant.
