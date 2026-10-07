# Gaviero

Terminal editor + headless CLI for AI agent orchestration. Rust 2024 workspace.

## Build & Test

### Build
```bash
cargo build                    # all crates
```
### Test
```bash
cargo test
```


Do not run rustfmt

Binaries: `gaviero` (TUI), `gaviero-cli` (headless), `gaviero-mcp-shim` (subprocess→MCP bridge).

## Workspace

Six crates — read the per-crate `CLAUDE.md` before touching its source.

- [`gaviero-core/`](crates/gaviero-core/CLAUDE.md) — all runtime logic; no UI/DSL deps.
- [`gaviero-tui/`](crates/gaviero-tui/CLAUDE.md) — terminal UI (ratatui + crossterm).
- [`gaviero-cli/`](crates/gaviero-cli/CLAUDE.md) — headless runner (clap).
- [`gaviero-dsl/`](crates/gaviero-dsl/CLAUDE.md) — `.gaviero` workflow compiler (logos + chumsky).
- [`gaviero-mcp-shim/`](crates/gaviero-mcp-shim/CLAUDE.md) — stdio↔socket bridge (Unix socket / Windows named pipe). Zero workspace deps.
- [`tree-sitter-gaviero/`](crates/tree-sitter-gaviero/CLAUDE.md) — `.gaviero` grammar.

Dependency rules: core has no UI/DSL deps. `tui` and `cli` depend on `core` + `dsl`. `dsl` depends on `core`. `gaviero-mcp-shim` is self-contained and reaches core only over the workspace MCP endpoint (`McpEndpoint`: `<workspace>/.gaviero/mcp.sock` on Unix, `\\.\pipe\gaviero-<hash>` on Windows). See [ARCHITECTURE.md](ARCHITECTURE.md) for the full topology.

## Architecture

Pipeline logic lives in `gaviero-core`. The TUI and CLI are thin wrappers that wire observers (`WriteGateObserver`, `AcpObserver`, `SwarmObserver` — [crates/gaviero-core/src/observer.rs](crates/gaviero-core/src/observer.rs)) to surface agent activity.

Subprocess coding agents (Claude Code, Codex, Cursor) reach core's in-process MCP server (read-only memory + graph tools) by spawning `gaviero-mcp-shim`, which pipes stdio to the workspace MCP endpoint (Unix socket / Windows named pipe — [crates/gaviero-core/src/mcp/transport.rs](crates/gaviero-core/src/mcp/transport.rs)). A loopback streamable-HTTP listener (`mcp.gavieroServer.http.enabled`, default on; bearer token in `.gaviero/mcp-http-token`) is advertised in `.gaviero/mcp-endpoint.json`; vendors switch to it per `mcp.gavieroServer.transport` / `transportByProvider` (default `stdio`). DeepSeek (`deepseek:`) stays an in-process fallback via [`tool_agent`](crates/gaviero-core/src/agent_session/tool_agent) + [`DeepseekBackend`](crates/gaviero-core/src/swarm/backend/deepseek.rs); it reaches `mcp.extraServers` through gaviero's own in-process MCP client ([`mcp/client.rs`](crates/gaviero-core/src/mcp/client.rs)), never a vendor config. `dsh:` speaks Agent Client Protocol to `dsh --profile acp` ([`agent_client_protocol`](crates/gaviero-core/src/agent_session/agent_client_protocol)) and mounts the gaviero server over that HTTP listener on `session/new` — it never uses the shim.

`.gaviero-workspace` files (any basename, fixed extension) describe multi-folder workspaces; bare directories are treated as single-folder workspaces. Dispatched at TUI startup in [crates/gaviero-tui/src/main.rs](crates/gaviero-tui/src/main.rs). `gaviero --workspace <dir>` opens the workspace file already in `<dir>`, or has the first-run wizard ([crates/gaviero-tui/src/setup.rs](crates/gaviero-tui/src/setup.rs)) build `<dirname>.gaviero-workspace` from the sub-folders the user picks. That wizard is the only place initial configuration is authored — it runs before `App` exists and writes `.gaviero/settings.json` from an agent profile (full / restricted, the latter dropping `Bash` from `agent.availableTools`), then optionally synthesizes the Claude/Codex/Cursor MCP configs. A `gaviero-cli` run never creates `.gaviero/settings.json`: it finds the workspace the TUI would open by walking up from its run root ([crates/gaviero-cli/src/state.rs](crates/gaviero-cli/src/state.rs)) and shares its memory, or runs on throwaway state with `--isolated` (also the fallback when no workspace is found).

Tier overrides for DSL scripts live in `examples/profiles/*.gaviero` (`doc-claude`, `doc-codex`, `doc-cursor`) and are loaded via `gaviero-cli --tiers-file <path>` (`tier <alias> <client>` lines only — [crates/gaviero-dsl/src/tiers.rs](crates/gaviero-dsl/src/tiers.rs)).

### Agent Runtime Parity

All interactive coding providers — Claude Code, Codex, Cursor, Ollama, DeepSeek (`deepseek:` and `dsh:`) — must expose the same user-facing contract:

- **Observable while running.** Reasoning deltas, tool starts, streaming status, file-proposal summaries, completion, and token usage flow through `AcpObserver` (or the swarm `UnifiedStreamEvent` adapter — [crates/gaviero-core/src/swarm/backend/mod.rs](crates/gaviero-core/src/swarm/backend/mod.rs)).
- **File edits never bypass review.** In TUI chat the host captures the turn, not the tools: [`turn_capture`](crates/gaviero-core/src/turn_capture/mod.rs) snapshots the workspace folders before the agent starts and diffs them after the session closes (no git — `.gitignore` is read as a plain pattern file, plus `files.exclude`; `.gaviero/` and `.git/` skipped; `.gaviero/settings.json` and sensitive paths always scanned). Every provider writes freely during the turn (`AgentOptions::host_capture` skips their own snapshot/revert/propose paths and pre-approves `Write`/`Edit`/`MultiEdit`); Bash keeps `ToolPolicy`. Whatever changed — edit tools, Bash, formatters, deletes, renames — becomes a `TurnChangeSet`, sensitive paths are auto-reverted, and the conversation is blocked until the TURN REVIEW is done (accept or reject each file, or the whole turn; reject = back to the pre-prompt version). If the baseline cannot be taken, the turn falls back to the provider-side Write Gate path. Swarm and `gaviero-cli` keep the Write Gate: native tools write inside their worktree, `dsh:` reconciles out-of-band writes via git dirty-set, Ollama's `<file path="relative/path">…</file>` blocks are extracted by [crates/gaviero-core/src/acp/protocol.rs](crates/gaviero-core/src/acp/protocol.rs).
- **One review path per entry point.** TUI chat: the turn change set ([crates/gaviero-tui/src/app/turn_review.rs](crates/gaviero-tui/src/app/turn_review.rs)). Swarm / CLI: `write_gate::WriteGatePipeline` ([crates/gaviero-core/src/write_gate.rs](crates/gaviero-core/src/write_gate.rs)).
- **Scope enforcement.** Proposals are checked against the active `FileScope` ([crates/gaviero-core/src/scope_enforcer.rs](crates/gaviero-core/src/scope_enforcer.rs)) before they leave the gate.
- **MCP routes every effect through the writer task.** Nine tools ([crates/gaviero-core/src/mcp/tools.rs](crates/gaviero-core/src/mcp/tools.rs)). Eight read-only: `memory_search`, `memory_get`, `memory_ping`, `blast_radius`, `node_doc`, `repo_outline`, plus `symbol_search` / `symbol_doc` behind `repoMap.symbolEnrichment.enabled`. `memory_ping` writes only to an in-memory probe ledger. One write-adjacent: `memory_flag` (`mcp.flag.enabled`, default true) demotes a stale memory's trust — it creates and deletes nothing, refuses user-authored and History rows, is idempotent, and every applied flag writes a reversible audit row. It reaches the writer through the narrow [`MemorySignalSink`](crates/gaviero-core/src/mcp/signal.rs), so `mcp/server.rs` still holds no `WriterHandle`.

## Conventions

- **Model spec is `provider:model`.** Bare names are rejected at dispatch (`validate_model_spec`, [crates/gaviero-core/src/swarm/backend/shared.rs](crates/gaviero-core/src/swarm/backend/shared.rs)). Prefixes: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:`, `dsh:`.
- **Lock discipline.** Never hold a `Mutex` across I/O, parsing, or embedding. The memory `writer` task is the single owner of SQLite writes.
- **Two-layer graph context.** The pre-prompt assembler injects `<repo_topology>` (shallow filesystem-only folder map, [crates/gaviero-core/src/repo_map/topology.rs](crates/gaviero-core/src/repo_map/topology.rs)) plus `<repo_outline>` (PageRank-ranked code outline). The TUI `/lite` chat command drops `<repo_outline>` + memory + impact and keeps only topology.
- **Plan production.** When drafting implementation plans for other agents, assume Claude Code (`claude:fable` / `claude:opus`) or Codex (`codex:gpt-5.6-sol`) unless the user will implement themselves. Plans must be agent-executable: concrete work units, ownership boundaries, expected files/modules, verification steps, sequencing constraints. Example client roster: [crates/gaviero-dsl/examples/clients.gaviero](crates/gaviero-dsl/examples/clients.gaviero). Drafting is separate from *consuming*: a stored plan binds only while it is being executed — see the `plans/` rule under Rules.

## Rules

- Every file change made during a TUI chat turn is captured in that turn's change set (`turn_capture`) and must be reviewed before the conversation continues. Swarm and CLI writes go through `WriteGatePipeline`. Host-side writes made during a turn (editor saves, review reverts, MCP config synthesis) must be recorded in `TurnCapture::ledger()` or they are attributed to the agent.
- Never let an MCP tool touch a store directly. Any tool with an effect goes through the writer task or the Write Gate — that is the invariant. A read-only surface is the default posture, not a hard rule: each tool costs ~150–250 prompt tokens on every subprocess turn (provenance: `git show f11eca9:tier-a-part-2-surface.md` §A5), so make it earn that.
- Never hold a `Mutex` across `.await`, embeddings, or filesystem I/O.
- Never emit a bare model name; always `provider:model`.
- Never edit `tree-sitter-gaviero/src/parser.c` or `grammar.json` by hand — regenerate from `grammar.js`.
- There is no `--no-memory` CLI flag; do not document or invent one — check [`Cli`](crates/gaviero-cli/src/main.rs) before adding flag docs.
- **`plans/`, `docs/plans/`, and `research/` are history, not spec.** A document in those folders is valid context **only** when (a) the task at hand is actually executing that plan — an instruction to continue, finish, or implement that line of work counts even if no path is named — or (b) the prompt names that document. In every other case it carries no authority: a newer prompt may contradict it outright, and the prompt wins. When you spot such a contradiction, say so — name the file, quote the claim the prompt overturns, then do what the prompt asked. Never silently reshape a request to match an older plan, never cite one of these files as the reason a change is required unless the prompt invoked it, and never treat one as a locked decision. The rule governs *authority, not access* — read whatever you need to do the job. Same rule for `CLAUDE.original.md` / `*.original.md` archives.

## Dependencies

Shared versions live in [`[workspace.dependencies]`](Cargo.toml) (tokio, serde, clap, reqwest, logos, chumsky, miette, …) — inherit with `{ workspace = true }`; add a crate-local version only for crate-specific deps. `tree-sitter 0.25` enters the graph exactly once, through `gaviero-core`'s re-exports ([crates/gaviero-core/src/lib.rs](crates/gaviero-core/src/lib.rs)) — never depend on it directly. Per-crate dependency lists live in each crate's CLAUDE.md.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — workspace-wide design, six-phase swarm pipeline, memory pipeline, MCP topology.
- [README.md](README.md) — user-facing feature reference.
