# gaviero-core

All runtime logic: swarm orchestration, memory, MCP server, ACP/agent sessions, write gate, scope + validation gates, git, terminal, repo-map, skills. **No UI or DSL dependencies.**

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
cargo test -p gaviero-core --features api-embedders   # placeholder factory only
```

Network/model tests (Ollama, embedder downloads, Cursor/Codex/Claude CLI presence) are `#[ignore]`.

## Architecture

**25 pub mods** — enumerate from [`src/lib.rs`](src/lib.rs). Orientation (not a substitute for reading the modules):

| Area | Entry | Notes |
|---|---|---|
| Swarm | [`swarm/`](src/swarm) | Six-phase pipeline; backends in [`swarm/backend/`](src/swarm/backend): `claude_code`, `codex`, `cursor`, `ollama`, `deepseek`, `mock`, `Custom` — all behind [`AgentBackend`](src/swarm/backend/mod.rs). |
| Agent session | [`agent_session/`](src/agent_session) | `claude`, `codex_exec`, `codex_app_server`, `cursor`, `ollama`, [`tool_agent/`](src/agent_session/tool_agent) (`deepseek:` + future API providers), `registry`. |
| Memory | [`memory/`](src/memory) | Multi-DB ONNX store; single writer task ([`writer.rs`](src/memory/writer.rs)); merged multi-scope hybrid retrieval (RRF). |
| MCP | [`mcp/`](src/mcp) | Seven read-only tools ([`tools.rs`](src/mcp/tools.rs)); endpoint via [`transport.rs`](src/mcp/transport.rs); config synth / preflight / telemetry. |
| Write path | [`write_gate.rs`](src/write_gate.rs), [`scope_enforcer.rs`](src/scope_enforcer.rs) | Modes: Interactive / AutoAccept / Deferred / RejectAll. |
| Repo map | [`repo_map/`](src/repo_map) | Graph + [`topology.rs`](src/repo_map/topology.rs) + symbol enrichment/search. |
| Skills | [`skills/`](src/skills) | Frontmatter, catalog, planner `ResolvedSkill` seam. |
| Other | `acp`, `context_planner`, `validation_gate`, `git`, `git_conflict`, `terminal`, `util`, `workspace`, … | See `lib.rs`. |

`tree-sitter` types are re-exported here; downstream crates **must not** depend on `tree-sitter` directly.

**DeepSeek path:** `deepseek:<id>` → [`BackendConfig::Deepseek`](src/swarm/backend/mod.rs) → [`DeepseekBackend`](src/swarm/backend/deepseek.rs) → [`tool_agent`](src/agent_session/tool_agent). Writes use Option-B `<file>` blocks through the Write Gate — same parity contract as Ollama stream providers. Allowed model ids: `DEEPSEEK_API_MODELS` in [`shared.rs`](src/swarm/backend/shared.rs).

**MCP endpoint:** `<workspace>/.gaviero/mcp.sock` (Unix) or `\\.\pipe\gaviero-<hash>` (Windows). Subprocess agents reach it via `gaviero-mcp-shim`. DeepSeek does not use the shim.

**Memory defaults:** embedder `nomic-embed-text-v1.5` (`memory.embedder.model = "nomic"`); symbol vectors `jina-code`; retrieval RRF (vector 0.7 + FTS 0.3); cascade mode is a kill-switch (`memory.retrieval.mode = "cascade"`).

## Conventions

- Lock discipline: never hold `Mutex` across I/O, parsing, or embedding. The memory writer task is the **single** owner of SQLite writes.
- `AgentBackend` is object-safe; every backend in [`swarm/backend/`](src/swarm/backend) implements it.
- Memory writes require explicit `WriteScope` — never infer. All writes flow through the writer task.
- Scoring ([`memory/scoring.rs`](src/memory/scoring.rs)): 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust. Decay-exempt types: `Decision` / `Convention` / `Invariant` / `Preference`.
- Model spec is `provider:model`. `validate_model_spec` ([`swarm/backend/shared.rs`](src/swarm/backend/shared.rs)) rejects bare names. Prefixes: `claude`, `codex`, `cursor`, `ollama`, `local`, `deepseek` (`SUPPORTED_PROVIDER_PREFIXES`).
- Tree-sitter access goes through `gaviero_core::{Language, Parser, Query, …}` re-exports.

## Rules

- **MCP tools are read-only.** Never add a write tool to [`mcp/tools.rs`](src/mcp/tools.rs); route writes through the Write Gate or the memory writer task.
- **No UI deps.** Compiles without `ratatui` / `crossterm`. `vt100`/`portable-pty` are allowed (embedded terminal lives here).
- **No DSL deps.** Must not depend on `gaviero-dsl`.
- **History rows are immutable** except via the C2.4 redaction path (`forget_history` requires explicit literal-string confirm + reason).
- **Decay-exempt types** must not be aged out by sleeptime.

## Dependencies

- `tree-sitter 0.25` + 16 grammars (incl. `tree-sitter-gaviero`).
- `git2 0.19` — worktrees, branches, diff.
- `rusqlite 0.32` (bundled) + `sqlite-vec 0.1.8` — memory store.
- `ort 2.0.0-rc.12` + `tokenizers 0.21` + `ndarray 0.17` — ONNX inference. `tokenizers` without default `esaxx_fast` (CRT `/MT` vs ort `/MD` on windows-msvc — see [Cargo.toml](Cargo.toml)).
- `rustdoc-types 0.57` + `syn 2` — rustdoc-JSON symbol enrichment.
- `petgraph 0.8` — swarm DAG.
- `portable-pty 0.9` + `vt100 0.16` — terminal emulation.
- `rmcp 1.5` + `schemars 1.2` — in-process MCP server.
- `zstd 0.13` + `bincode 1.3` — History compression.
- `windows-sys 0.59` (Windows only) — kill-on-close Job Objects ([`util::spawn`](src/util/spawn.rs)).
- `reqwest`, `async-trait`, `futures`, `tokio-stream`, `tokio-util`, `chrono`, `regex`, `walkdir`, `toml`, `tempfile`, `ropey`, `similar`, …
- Dev: `wiremock 0.6`, `insta 1` ([tests/snapshots/](tests/snapshots)).

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — module map, swarm/memory pipelines, MCP topology, write-gate flow.
- [README.md](README.md) — public-API reference.
- [../../ARCHITECTURE.md](../../ARCHITECTURE.md) — workspace-wide design.
