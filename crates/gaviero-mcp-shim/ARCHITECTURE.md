# gaviero-mcp-shim — Architecture

Standalone stdio↔endpoint bridge. Subprocess coding agents (Claude Code, Codex, Cursor) spawn this binary as their MCP "server"; it connects to Gaviero's workspace [`McpEndpoint`](../gaviero-core/src/mcp/transport.rs) and copies bytes bidirectionally. Protocol handling lives in [`GavieroMcpServer`](../gaviero-core/src/mcp/server.rs) inside the host.

Binary: `gaviero-mcp-shim` (~187 lines, single source file). Conventions: [CLAUDE.md](CLAUDE.md).

---

## Topology

```
Subprocess agent (claude / codex / cursor)
        │ stdin/stdout  JSON-RPC 2.0 (MCP)
        ▼
gaviero-mcp-shim
  connect_with_backoff → bridge (tokio::io::copy ×2)
        │
        ▼  Unix: <workspace>/.gaviero/mcp.sock
           Windows: \\.\pipe\gaviero-<hash>
gaviero-core::mcp::server  (rmcp, seven read-only tools)
```

**Zero workspace deps.** Only `tokio`, `clap`, `anyhow`, `tracing`. Speaks to core exclusively over the endpoint — never links `gaviero-core`.

DeepSeek (`deepseek:`) runs in-process via `tool_agent` and does **not** use this shim.

---

## Modules

| Path | Role |
|---|---|
| [`src/main.rs`](src/main.rs) | `Cli`, `connect_with_backoff` (unix/windows), `bridge`, `main` |
| [`Cargo.toml`](Cargo.toml) | Standalone manifest |

---

## Abstractions

### `Cli`

```rust
struct Cli {
    socket: Option<PathBuf>,          // --socket (Unix)
    pipe: Option<String>,             // --pipe (Windows)
    connect_timeout_secs: u64,        // default 5
}
```

Args emitted by [`McpEndpoint::shim_args`](../gaviero-core/src/mcp/transport.rs) into synthesized agent configs ([`config_synth`](../gaviero-core/src/mcp/config_synth.rs)).

### `connect_with_backoff`

Retries `UnixStream::connect` / Windows named-pipe open (folding `ERROR_PIPE_BUSY`) with exponential backoff (50 ms → 400 ms) until success or deadline. Lets the agent spawn before the host finishes binding.

### `bridge`

Splits the connected stream; two tasks under `tokio::select!`:

- stdin → endpoint (flush after each chunk)
- endpoint → stdout (flush)

Fixed 8192-byte buffers. Byte-faithful — no framing or JSON parsing.

---

## Data Flow

```
Agent JSON-RPC request line
  → stdin → buffer → endpoint write + flush
  → GavieroMcpServer executes tool (memory_search | memory_get |
       blast_radius | node_doc | repo_outline | symbol_search | symbol_doc)
  → response line → stdout + flush
  → Agent
```

Tool list and semantics: [`crates/gaviero-core/ARCHITECTURE.md`](../gaviero-core/ARCHITECTURE.md) (MCP section) / [`mcp/tools.rs`](../gaviero-core/src/mcp/tools.rs). All tools are read-only.

---

## Concurrency

Single-thread tokio runtime. Two concurrent copy tasks; first EOF/error wins via `select!`. No shared state, no locks.

---

## Error Handling

| Failure | Handling |
|---|---|
| Endpoint not ready | Backoff until `--connect-timeout-secs`, then exit with annotated `io::Error` |
| Either direction closes | Propagate via `anyhow::Context`; peer task dropped |
| Tracing | stderr at WARN — stdout reserved for JSON-RPC |

No per-request retries (agent responsibility).

---

## API

None (binary). Agents launch via synthesized MCP config:

- Claude: `<worktree>/.mcp.json` — [`claude_mcp_config_json`](../gaviero-core/src/mcp/config_synth.rs)
- Codex: `<worktree>/.codex/config.toml` — [`codex_mcp_config_toml`](../gaviero-core/src/mcp/config_synth.rs)
- Cursor: `<worktree>/.cursor/mcp.json` — [`cursor_mcp_config_json`](../gaviero-core/src/mcp/config_synth.rs)

`command` must resolve `gaviero-mcp-shim` on `PATH` or use an absolute path.
