# gaviero-mcp-shim — Architecture

A standalone stdio↔Unix-socket bridge. Subprocess coding agents (Claude Code, Codex, Cursor) spawn this binary as their MCP "server"; it opens a connection to `<workspace>/.gaviero/mcp.sock` and pipes bytes in both directions. The actual MCP protocol is handled by [`GavieroMcpServer`](../gaviero-core/src/mcp/server.rs) inside the host process.

Binary: `gaviero-mcp-shim` (~110 lines of Rust, single source file)

---

## 1. Topology

```
┌────────────────────────────┐
│ Subprocess agent           │
│ (claude-code / codex /     │
│  cursor)                   │  ─── stdin/stdout (JSON-RPC 2.0)
└────────────────┬───────────┘
                 │
                 ▼
┌────────────────────────────┐
│ gaviero-mcp-shim           │
│                            │
│  ┌──────────────────────┐  │
│  │ connect_with_backoff │  │
│  └──────────┬───────────┘  │
│             ▼              │
│  ┌──────────────────────┐  │
│  │ bridge (tokio::io::  │  │
│  │   copy bidirection)  │  │
│  └──────────┬───────────┘  │
└─────────────┼──────────────┘
              │
              ▼   <workspace>/.gaviero/mcp.sock (Unix socket)
              │
┌─────────────┴──────────────┐
│ gaviero-core::mcp::server  │
│ (in-process rmcp server)   │
│   memory_search            │
│   blast_radius             │
│   node_doc                 │
└────────────────────────────┘
```

**Standalone crate.** No workspace dependencies — only `tokio`, `clap`, `anyhow`, `tracing`. The shim builds and ships independently of the rest of the workspace.

---

## 2. Modules

The crate is a single source file:

| File | Purpose |
|---|---|
| [`src/main.rs`](src/main.rs) | `Cli` (clap derive), `connect_with_backoff`, `bridge`, `main` |
| [`Cargo.toml`](Cargo.toml) | Standalone manifest — no `gaviero-core` / `gaviero-dsl` deps |

---

## 3. Core Abstractions

### `Cli` ([`src/main.rs`](src/main.rs))

```rust
struct Cli {
    /// Absolute path to <workspace>/.gaviero/mcp.sock
    socket: PathBuf,
    /// Retry window for the initial connect (default 5s)
    connect_timeout_secs: u64,
}
```

### `connect_with_backoff` ([`src/main.rs`](src/main.rs))

Retries `UnixStream::connect` with exponential backoff (50 ms → 400 ms ceiling) until either the connection succeeds or the deadline (`Instant::now + connect_timeout_secs`) passes. Used so the subprocess can spawn before the host finishes `Workspace::open`.

### `bridge` ([`src/main.rs`](src/main.rs))

Splits the connected `UnixStream` into `(sock_rx, sock_tx)`, then runs two async tasks under `tokio::select!`:

- `to_sock`: `stdin → sock_tx` with explicit `flush()` after every chunk.
- `from_sock`: `sock_rx → stdout` with explicit `flush()`.

Both tasks use a fixed 8192-byte buffer. The first task to return EOF or an error terminates the bridge.

---

## 4. Data Flow — One Tool Call

```
Agent sends JSON-RPC request line (memory_search …)
   │
   ▼
stdin → 8192-byte buffer → sock_tx.write_all → flush
   │
   ▼
.gaviero/mcp.sock
   │
   ▼
rmcp server in gaviero-core (executes memory_search,
   returns JSON-RPC response line)
   │
   ▼
sock_rx → 8192-byte buffer → stdout.write_all → flush
   │
   ▼
Agent reads response
```

MCP over stdio is line-delimited JSON-RPC 2.0; the shim is byte-faithful (no framing, no parsing) — exactly what `rmcp` expects on either end.

---

## 5. Concurrency

- Single-thread `tokio` runtime (`#[tokio::main]` default).
- Two concurrent `async` tasks inside `bridge`; `tokio::select!` ensures the first to finish drops the other.
- No shared state, no locks. Each direction owns its half of the split stream.

---

## 6. Error Handling

| Failure | Handling |
|---|---|
| Socket missing / host not yet started | Retry with exponential backoff until `connect_timeout_secs` elapses, then exit with the underlying `io::Error` annotated by `anyhow::Context` |
| Either pipe direction closes | Return cleanly; `tokio::select!` propagates the error wrapped with `.context("piping stdin → socket")` / `.context("piping socket → stdout")` |
| `tracing` | Logged to stderr at `WARN` level — keeps stdout clean for JSON-RPC traffic |

The shim never invents framing or retries individual requests — that's the agent's job.

---

## 7. Public API

None — this is a binary crate. Subprocess agents launch it via their MCP config:

- Claude Code: `<worktree>/.mcp.json`'s `gaviero` server entry (see [`claude_mcp_config_json`](../gaviero-core/src/mcp/config_synth.rs)).
- Codex: `<worktree>/.codex/config.toml`'s `[mcp_servers.gaviero]` block (see [`codex_mcp_config_toml`](../gaviero-core/src/mcp/config_synth.rs)).
- Cursor: `<worktree>/.cursor/mcp.json` (same schema as Claude, see [`cursor_mcp_config_json`](../gaviero-core/src/mcp/config_synth.rs)).

Each config sets `command` to `gaviero-mcp-shim` and passes `--socket <abs-path>`. The shim must therefore be on `PATH` or referenced by absolute path; otherwise the agent's MCP startup fails.

---

## 8. Relationship to `gaviero-core`

The shim is the only piece of glue that lets out-of-process agents reach the in-process MCP tools. The tools themselves — `memory_search`, `blast_radius`, `node_doc` — are read-only by construction (the server has no `WriterHandle`). See [`crates/gaviero-core/ARCHITECTURE.md`](../gaviero-core/ARCHITECTURE.md) §7 for the server side.

---

See [README.md](README.md) for installation and usage examples.
