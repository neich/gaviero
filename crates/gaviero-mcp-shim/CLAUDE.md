# gaviero-mcp-shim

A small stdio↔socket bridge (~200 lines). Subprocess coding agents (Claude Code, Codex, Cursor) spawn this binary as their MCP "server"; it opens a connection to Gaviero's workspace endpoint — a Unix domain socket on Unix, a named pipe on Windows — and pipes bytes in both directions. The actual MCP protocol terminates at the in-process rmcp server inside [`gaviero-core`](../gaviero-core/CLAUDE.md).

Binary: `gaviero-mcp-shim` ([src/main.rs](src/main.rs)).

## Build & Test

```bash
cargo build -p gaviero-mcp-shim --release
cargo install --path crates/gaviero-mcp-shim   # put on PATH for subprocess agents
```

Subprocess MCP configs (`<worktree>/.mcp.json` for Claude Code, `<worktree>/.codex/config.toml` for Codex, `<worktree>/.cursor/mcp.json` for Cursor) reference the shim by name — install it on `PATH` or use an absolute path in the synthesised config. Per-worktree configs are written by [`gaviero_core::mcp::config_synth`](../gaviero-core/src/mcp/config_synth.rs).

## Architecture

- `connect_with_backoff` (per-platform `unix`/`windows` modules) — retries the endpoint connect with exponential backoff (50 ms → 400 ms) until the deadline, so the shim survives Gaviero restarting `Workspace::open` after the subprocess is already spawned. The Windows arm folds `ERROR_PIPE_BUSY` into the same retry loop.
- `bridge` — generic over `AsyncRead + AsyncWrite` halves: bidirectional `tokio::select!` between stdin→endpoint and endpoint→stdout using byte-faithful copy loops. Exits when either side closes.
- MCP over stdio is line-delimited JSON-RPC 2.0; the shim does **not** parse or reframe — `rmcp` on the server side expects byte-faithful delivery.

## Flags

| Flag | Default | Purpose |
|---|---|---|
| `--socket <path>` | (required on Unix) | Absolute path to the workspace MCP socket (`<workspace>/.gaviero/mcp.sock`). Unix only. |
| `--pipe <name>` | (required on Windows) | Named-pipe name (`\\.\pipe\gaviero-…`). Windows only. |
| `--connect-timeout-secs <N>` | `5` | Total seconds the initial connect will retry before failing. |

`tracing-subscriber` is initialised at WARN level on stderr.

## Conventions

- **Zero workspace dependencies.** The shim links only `tokio`, `clap`, `anyhow`, `tracing`, `tracing-subscriber`. It does **not** depend on `gaviero-core`, `gaviero-dsl`, or any other workspace crate. This keeps the binary small (a few KB) and makes `.mcp.json`'s `command` field resolve cleanly everywhere.
- **Byte-faithful piping.** Never parse, log, or transform the MCP traffic — preserving line boundaries is the contract.
- **Stderr-only logging.** Stdout is reserved for MCP responses.

## Rules

- **Do not pull in workspace deps.** If a feature seems to require `gaviero-core`, it belongs on the server side instead.
- **Do not write to stdout** outside the byte-faithful socket→stdout loop. Any diagnostic must go to stderr through `tracing`.
- **Connect retries are bounded.** Past the deadline, return the underlying `io::Error` wrapped with `Context` — never silently keep retrying.
- **The shim has no MCP awareness.** Tool semantics, schemas, and access control live in [`gaviero_core::mcp`](../gaviero-core/src/mcp). The shim is a dumb pipe; keep it that way.

## Dependencies

- `tokio` (full) — async runtime, `UnixStream`, stdio.
- `clap` (derive) — `--socket` / `--connect-timeout-secs`.
- `anyhow` — error context on the connect/copy boundary.
- `tracing` + `tracing-subscriber` — stderr WARN logger.

## See Also

- [`gaviero_core::mcp`](../gaviero-core/src/mcp) — server side: tools, config synth, observer, external-memory detection.
- [`../../CLAUDE.md`](../../CLAUDE.md) — workspace overview and the MCP "read-only by construction" invariant.
