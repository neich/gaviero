# Tier A Part 2 — Memory Surface (Panel and MCP Server)

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, call sites, data flow, task ordering, test strategy, acceptance criteria, and open decisions the developer must make. Flag decisions that require human judgment rather than deciding them yourself. Do not produce full Rust code listings — trait signatures, struct skeletons, and pseudo-code snippets are acceptable where they reduce ambiguity.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, ratatui, and the Model Context Protocol (MCP). Do not explain these.

**This document covers Phase 2 of Tier A**, containing two items (A4, A5). Phase 1 (A3, A2, A1) is covered in a separate document (**Tier A Part 1 — Schema, Scope, and Signal**) and is a **prerequisite**. Treat Phase 2 as self-contained for planning, but do not start implementing until Phase 1 has landed — the panel displays fields and the MCP server exposes data that Phase 1 creates.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (a full-screen terminal editor built on ratatui) and `gaviero-cli` (headless runner). It orchestrates upstream coding agents — Claude Code via ACP, Codex via `codex exec` and `codex-app-server`, Ollama via HTTP SSE, and a mock backend — through a provider-agnostic `AgentBackend` trait producing `UnifiedStreamEvent`s. Agent writes to disk flow through a `WriteGatePipeline` for interactive diff review. Multi-agent work is orchestrated via a 6-phase swarm pipeline: validate → execute → merge → verify → cleanup → consolidate.

It is **local-first** and **single-developer-oriented**. No SaaS backend, no cloud persistence, no multi-user concerns.

### Stack

- Rust 2024, tokio, petgraph, ropey, git2, crossterm, ratatui 0.30
- SQLite (WAL mode) + sqlite-vec for vector storage
- ONNX Runtime (via `ort` crate) with nomic-embed-text-v1.5 (768 dim) for local embedding
- tree-sitter 0.25 with a 16-language registry
- portable-pty for embedded terminal
- logos + chumsky + miette for the `.gaviero` DSL compiler

### Crate layout

- `gaviero-core` (lib, 21 public modules) — all runtime logic, no UI
- `gaviero-tui` (bin `gaviero`) — ratatui UI, event routing, observer implementations
- `gaviero-cli` (bin `gaviero-cli`) — single-file clap CLI
- `gaviero-dsl` (lib) — `.gaviero` workflow script compiler
- `tree-sitter-gaviero` — grammar for `.gaviero` files

### Memory subsystem (current state, assuming Tier S and Phase 1 of Tier A are in place)

- 5-level scope hierarchy: `Global → Workspace → Repo → Module → Run`
- One SQLite WAL file per workspace (`<workspace>/.gaviero/memory.db`) + a separate global file
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- Cascading retrieval with early exit at 0.70 (RRF 0.7 vec / 0.3 fts) — Tier B revises
- Composite scoring: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust_multiplier`
- SHA-256 content hash dedup, explicit `WriteScope`
- **Writer task** owns all writes (Tier S2); all subsystems enqueue `WriterMessage` variants
- **Chat auto-inject** in place (Tier S1); default scope set `Workspace ∪ Repo ∪ Module`
- **Per-turn extractor** in place (Tier S3); emits 0–5 candidates per turn via the writer task
- **Retrieval manifest persistence** (Tier S4): every injection produces a persistent `injection_manifests` row; 30-day retention
- **`source` and `trust` attributes on all records** (Phase 1 A3): fully plumbed; composite score multiplies by trust
- **`/remember` scope-corrected** (Phase 1 A2): defaults to Repo; explicit variants for each scope level; badge confirmations
- **Turn annotations sidecar** (Phase 1 A1): LLM emits `<turn_annotations>` JSON; flags feed writer task as `source = "llm_annotated"`, `trust = 0.7`; `session_thread` and `open_questions` stored in session-ledger table

### Agent session / prompt construction

`agent_session::build_turn` produces a `Turn` from `PlannerSelections`. Chat goes through `acp::client::AcpPipeline::send_prompt`. Subprocess coding agents are spawned by the swarm pipeline or by direct chat; both run Claude Code (via ACP) or Codex (via `codex exec` or `codex-app-server`) or Ollama.

### Hard constraints (non-negotiable)

1. All agent writes flow through `WriteGatePipeline`.
2. `git2` only.
3. Tree-sitter for all syntax work.
4. `MemoryStore` behind `tokio::sync::Mutex`; embedding outside lock.
5. **No Mutex across `await`, parse, or `fs` I/O.**
6. `gaviero-core` has no UI/CLI types; coupling via observer traits.
7. Single TUI `mpsc::UnboundedReceiver<Event>` main loop; no background task mutates `App`.
8. Provider-agnostic `AgentBackend`.
9. Config via `settings.json` cascade.
10. Memory writes require explicit `WriteScope`.
11. **All memory writes go through the Tier S2 writer task.** Panel edits enqueue `PanelEdit` variants; never touch `MemoryStore` directly.
12. **MCP surface is read-only.** Writes never go through MCP.

### Decisions already taken (do not re-litigate)

- Writer task + annotations sidecar + read-only MCP + three-cadence consolidation.
- No LLM writes; no graph-RAG / Neo4j / A-MEM memory evolution.
- Keep nomic-embed-text-v1.5 for now (Tier B swaps).

### What Tier S and Phase 1 of Tier A delivered (assume in place)

- Chat auto-injects memory at prompt assembly time (Tier S1)
- Writer task with `WriterMessage` enum: `UserRemember`, `TurnComplete`, `SwarmConsolidate`, `InjectionManifest`, `PanelEdit` (Tier S2; `PanelEdit` stub-defined, consumer is A4)
- Per-turn extractor (Tier S3)
- Retrieval manifest persistence (Tier S4)
- `source` and `trust` on every record (Phase 1 A3)
- `/remember` with scope-corrected default and visible badges (Phase 1 A2)
- Turn annotations sidecar with `session_thread` and `open_questions` (Phase 1 A1)
- `MemoryObserver` trait with `on_write_enqueued`, `on_write_committed`, `on_write_failed`
- `ManifestObserver::on_manifest_persisted(turn_id)` callback
- `AcpObserver::on_memory_injected(summary)` callback
- Settings schema: `memory.chatInjection.*`, `memory.extractor.*`, `memory.manifests.*`, `memory.remember.*`, `memory.annotations.*`

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle. Phase 2 of Tier A is pure surface — the TUI panel (A4) makes the Memory layer inspectable and user-curatable; the MCP server (A5) exposes read-only views of Memory + KG to subprocess agents. Neither touches the History layer directly (that becomes explicit in Tier C1). Both depend heavily on Phase 1 data: the panel displays `source`, `trust`, and annotation-derived records; the MCP server would be nearly useless without the trust gradient filtering noise.

---

## Phase 2 context and rationale

Phase 2 exposes Gaviero's memory to two different audiences: the user (via a TUI panel) and subprocess coding agents (via a read-only MCP server). Both depend on Phase 1's schema additions and on Tier S's manifest infrastructure.

- **A4 (TUI memory panel)** — Makes memory inspectable. No production coding agent does this well; it is Gaviero's differentiator. Also the key debugging tool for every later tier.
- **A5 (Gaviero-as-MCP-server, read-only)** — Extends memory reach into subprocess coding agents. Replaces third-party MCP memory servers with a single source of truth.

**Dependencies and ordering within Phase 2:**

- **A4 should ship before A5.** The panel is how you'll debug MCP tool calls (A5 emits `McpToolCallObserver` events that the panel displays). Without A4 in place, A5 is working blind.
- **A4 can ship incrementally** — start with the manifest-backed "Injected Now" section only; add Recently Written, Scope Summary, Search in subsequent sub-tasks.
- **A5 is the largest sub-task** in this phase due to the per-backend MCP config synthesis work.

**Recommended sequence:** A4 (Injected Now section first, then others) → A5.

---

## Items in this phase

### A4. TUI memory panel

**Problem.** Gaviero's memory is invisible to the user. The main signal they get today is response quality. When memory works, they can't confirm it; when it fails, they can't debug it. This is a UX gap no production coding agent has closed.

**Architecture.** A right-side TUI pane sharing space with the agent chat panel (the existing `SidePanelMode` enum in `gaviero-tui` gains a `MemoryPanel` variant). Toggled with `Alt+m`. Width 25–30% of terminal, same as agent chat.

Four sections, vertically stacked, scrollable independently, `Tab` cycles focus:

**1. Injected now (top, ~40% of pane height).**
The actual memories injected into the current chat turn's context. **Backed by the S4 retrieval manifest** for the current turn, read from `injection_manifests`. Subscribes to `ManifestObserver::on_manifest_persisted` for live updates. Each row:

```
[P] decision • "Chose tokio::sync::Mutex over std..." • 0.87 • trust 0.7
```

Scope badge (R/M/P/W/G), type, truncated text, composite score, trust. Most important section — makes retrieval transparent. When users think "why didn't it remember X?" they see immediately whether X was retrieved and scored below the cutoff or not retrieved at all.

**Actions in this section:**
- `i` → **Inspect manifest** — opens full candidate pool (typically 20–100 entries) with per-item score breakdown (sim, importance, recency, scope_mult, trust, composite) and exclusion reasons. Critical for retrieval debugging.
- `h` → **History** — navigates to previous turns' manifests (within 30-day retention window). Shows per-turn: query, selected set, pool size. Enables the user to walk backwards through retrieval decisions.

**2. Recently written (middle, ~25%).**
Memories created in the last 24 hours across all scopes. Populated via `MemoryObserver::on_write_committed`. Same row format plus source badge (e.g., `⟂U` for user, `⟂X` for extractor, `⟂A` for annotated, `⟂C` for consolidated). Key interactions:

- `d` → delete (with `y` confirmation); enqueues a `PanelEdit { op: Delete(id) }` writer message
- `p` → pin (raises trust to 1.0)
- `s` → change scope (opens sub-menu)
- `e` → edit text (opens input field, commits on Enter)

**3. Scope summary (middle, ~20%).**
Aggregate stats:
```
Run       │  12 │ last write 2s ago
Module    │  3  │ last write —
Repo      │ 147 │ last write 1h ago
Workspace │  42 │ last write 1d ago
Global    │  8  │ last write 4d ago
```
Plus last-consolidation-time per scope and a "health" indicator (red if no writes in N days for an active project).

**4. Search (bottom, ~15%).**
`/` activates a live fuzzy search input. Shows top-5 matches as the user types, using the same hybrid retrieval as chat injection. `Enter` on a result opens detail view; `Esc` clears.

**Interaction contract.**

- The panel is a **view over `memory.db`**, not a cache. Queries hit SQLite directly with `<10ms` refresh budget.
- Real-time updates: new injections highlight for 2 seconds; new writes animate in from top of Section 2.
- All destructive ops (delete, merge, scope change) require `y` confirmation and log to an audit table (foundation for Tier C2 `/forget`).
- Read-only by default; enter edit mode only on explicit keypress.
- All edits enqueue `WriterMessage::PanelEdit` variants; never touch `MemoryStore` directly.

**Benefits.**

- Differentiating feature. Makes memory a first-class workflow tool.
- Debugging accelerator. Every later memory feature is easier to evaluate with the panel running alongside.
- Addresses the "starts from scratch" complaint at the perceptual level — even before retrieval is perfect, users can *see* what's happening.
- Gives users a lightweight curation surface (delete bad, pin good) without needing a separate UI.

**Risks.**

- **UI complexity creep.** Four sections is the floor; resist expansion.
- **Performance if refresh isn't budgeted.** Use `<10ms` SQLite query budget; profile with `EXPLAIN QUERY PLAN` on every query the panel issues.
- **Edit mode bypass risk.** All edits must go through `PanelEdit` writer messages; a bug that lets the panel call `MemoryStore` directly violates the Tier S2 invariant.
- **Event storm.** Writing 20 memories in a 1-minute extractor burst could flood the animate-in queue. Debounce UI updates to 100ms granularity.
- **Manifest-inspection UX complexity.** 100-entry candidate pools with 6 score components each are information-dense. Plan the layout carefully — columns with sparkline-style score bars beat a wall of numbers.

**Alternatives considered and rejected.**

- *CLI-only `gaviero memory list/show` commands.* Sufficient for headless debugging, insufficient to make memory a workflow tool.
- *Web UI.* Out of scope for a TUI-first product.
- *Status-bar indicator with drill-down.* Too little surface area.

**Integration notes.**

- Add a `MemoryPanelState` struct in `gaviero-tui/src/panels/memory_panel.rs` (new file). Follows the pattern of `agent_chat.rs`.
- Keybinding `Alt+m` follows the existing `Alt+<letter>` side-panel convention.
- `MemoryObserver` and `ManifestObserver` callbacks from Tier S fire into a TUI event channel variant `Event::MemoryEvent(MemoryEventKind)` — do not mutate `App` from the observer directly.
- Section 1's data comes from `injection_manifests` (via `ManifestObserver`) rather than from an in-memory snapshot.
- Section 2's recency window (24h default) is configurable via `ui.memoryPanel.recentWindowHours`.
- When the TUI restarts, re-query `memory.db` for a bootstrap fill of the Recently Written section and the most recent manifest for Section 1.

---

### A5. Gaviero-as-MCP-server (read-only tool surface)

**Problem.** Subprocess coding agents (Claude Code, Codex) get memory only at prompt assembly time via Tier S1 injection. They cannot refine their own context mid-turn — ask for more memory, query blast radius, fetch a module's doc. External MCP memory servers duplicate state with their own stores. Gaviero should expose its own memory via a read-only MCP interface that replaces external servers.

**Architecture.**

- **In-process MCP server** running as a tokio task inside `gaviero-core`. Use the official `rmcp` crate (the same one Codex uses internally) or another Rust MCP SDK. Transport: **stdio only** for now (both Claude Code and Codex support stdio; streamable HTTP is unnecessary for local subprocess communication).
- Spawn pattern: the MCP server task starts at `Workspace::open` time and listens on a Unix domain socket (or on Windows, a named pipe) under `<workspace>/.gaviero/mcp.sock`. When the swarm or chat spawns a subprocess coding agent, Gaviero synthesizes a per-run MCP config:
  - For **Claude Code**: writes `.mcp.json` in the subprocess's working directory (worktree) with an entry pointing at Gaviero's socket via a small stdio-shim binary OR spawns a companion process that reads the socket and exposes stdio. Alternatively, pass `--mcp-config <path>` flag with the generated config.
  - For **Codex**: writes a project-scoped `.codex/config.toml` with `[mcp_servers.gaviero]` pointing at the stdio shim. Requires the worktree to be a trusted Codex project — one-time user consent dialog.
- **Tool surface (exactly three tools; read-only).**
  - `memory_search(query: string, scope_hint?: string, limit?: number) → { results: [{ scope, type, text, importance, trust, refs }] }`
  - `blast_radius(paths: [string], depth?: number, mode?: "impact" | "callers" | "tests" | "all") → { nodes: [{ path, relation, distance, purpose? }] }`
  - `node_doc(path: string) → NodeDoc` (schema defined in Tier D1; MVP returns what's available — purpose fields are empty strings until Tier D)
- Three tools, all read. `memory_store`, `memory_update`, `memory_delete` are **explicitly not exposed.** Writes go through annotations sidecar (Phase 1 A1) or the transcript extractor.
- Per-call trust: MCP calls originate from trusted local subprocesses; do not add auth. But do audit: every tool call emits a `McpToolCallObserver` event for the panel.

**Settings.**

- `mcp.gavieroServer.enabled: bool` (default `true`)
- `mcp.gavieroServer.exposedTools: [string]` (default all three; allows selective disable)
- `mcp.gavieroServer.disableExternalMemory: bool` (default `true`) — detect and disable `@modelcontextprotocol/server-memory`, `mem0-mcp`, `memory-bank-mcp` entries in user's MCP config, offering migration (migration import is a separate minor task)

**Benefits.**

- Gives subprocess agents mid-turn memory access, removing the "what was pre-injected is all I get" ceiling.
- Cleanly replaces external MCP memory servers with single source of truth.
- Makes blast-radius and KG queries available inline for multi-file refactors.
- Read-only design preserves Phase 1 A3 trust discipline by construction.

**Risks.**

- **Tool schema tokens.** Each MCP tool adds ~150–250 tokens to the subprocess's system prompt. Three tools → ~500–700 tokens every subprocess turn. Acceptable but measurable. Monitor.
- **Codex MCP support has been buggy.** Through late 2025, Codex has had MCP config detection issues in some transport modes. Test with current Codex version; have a fallback path (simply no MCP, fall back to prompt-time injection only).
- **Sandbox/trust concerns.** When Codex runs in workspace-write or full-access mode, it has access to `memory.db` anyway. MCP doesn't leak — it exposes the same data through a structured interface. But document this clearly.
- **Transport shim complexity.** If MCP SDK requires a separate process per server, you'll end up with a shim binary. Prefer SDKs that support in-process server implementations accessible via stdio without spawning.
- **Circular tool invocations.** If an MCP `memory_search` call is logged as a tool-output memory, and that memory shows up in the next injection, avoid infinite growth. Do not log MCP reads as memories; only log as observer events.

**Alternatives considered and rejected.**

- *Keep prompt-time injection only.* Loses the "agent notices it needs more" capability, which matters on long swarm tasks.
- *Expose writes via MCP.* Reintroduces LLM-as-writer failure mode that Cursor retreated from.
- *Custom protocol over ACP.* Works for Claude Code but not Codex; MCP is the lingua franca.
- *Hierarchical file-system namespace over MCP* (the Xu et al. 2025 "everything is a file" proposal). Considered: the AIGNE paper proposes a unified `afs_list`/`afs_read`/`afs_exec` namespace. Rejected for Gaviero: production coding agents (Claude Code, Codex, Cursor) converge on focused tool sets; the claim that LLMs navigate hierarchical namespaces better than they call typed tools is unsupported by evidence. Three explicit tools with typed schemas beat an open namespace until measurement shows otherwise.

**Integration notes.**

- Audit all user-facing MCP-related docs to make clear: "Gaviero exposes its own memory via MCP. External MCP memory servers are disabled by default." Include migration instructions for users who had data in external servers.
- Migration task (can be done as part of A5 or separately): detect `memory.jsonl` files from `@modelcontextprotocol/server-memory` and import at `Workspace` scope with `source = "mcp_import"`, `trust = 0.5` (uses the A3 trust/source schema).
- Observer event `McpToolCallObserver::on_tool_call(tool_name, input, output, duration_ms)` for panel display (A4 shows these in a sub-pane or overlays).
- The stdio shim (if needed) is a tiny separate binary `gaviero-mcp-shim` in the same cargo workspace. It connects to the workspace socket and proxies stdio.

---

## Expected output format from the implementation plan

For each of A4, A5 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from Tier S and Phase 1 of Tier A); recommended sub-task order within the item.

**3. Affected crates and modules.** Tree-style list of touched files.

**4. New types, traits, and public API.** High-level signatures; for A5 specifically call out the MCP tool JSON schemas and transport shim boundary.

**5. Data-flow description.** Plain prose or ASCII: which tasks, which channels, which locks. For A4: observer-to-TUI-event flow. For A5: socket-to-MCP-SDK-to-MemoryStore flow.

**6. Schema / settings / UI changes.** Settings.json additions; TUI keybindings and panel layout; for A5, per-backend MCP config file templates.

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies. A4 should be breakable into sub-sections shippable independently (start with Injected Now only).

**8. Test strategy.** Unit + integration + observer + UI-snapshot tests (for A4) + end-to-end MCP integration tests (for A5) that run each subprocess backend against a minimal task and verify tool calls succeed.

**9. Observability.** Tracing spans, metric counters, observer events. For A4: refresh latency per section; event-storm debounce rate. For A5: per-tool call rate, latency P95, failure rate, subprocess-config-synthesis failures.

**10. Risks and rollback.** Per sub-task: failure modes and reversal steps. For A5: what happens if Codex MCP config breaks a user's existing setup.

**11. Acceptance criteria.** Per item: measurable conditions testable by a human.

**12. Open questions.** Decisions requiring developer judgment. Examples: whether the panel's manifest-inspection view should support CSV export; whether Codex MCP config should be opt-in per workspace or global; whether to implement migration from `@modelcontextprotocol/server-memory` or just disable-and-warn; how to handle the stdio-shim binary distribution across the cargo workspace.

---

## Explicit non-goals for Phase 2

Do **not** plan the following here:

- Any retrieval algorithm changes — Tier B
- Embedding model swap — Tier B
- Cross-encoder reranker — Tier B
- Session consolidator and sleeptime pass — Tier B
- Retrieval-use telemetry — Tier B
- Typed memory stores split — Tier C
- `/forget` command with audit trail — Tier C (A4's delete operation lays groundwork but the full audit story is C2)
- PageRank or code graph changes — Tier C
- KG node-doc schema — Tier D (A5 exposes `node_doc` as a stub returning empty semantic fields)
- Contextual Retrieval — Tier D
- AGENTS.md / CLAUDE.md compat — Tier D
- Additional MCP tools beyond `memory_search`, `blast_radius`, `node_doc`
- Write-capable MCP tools — explicitly rejected

---

## Acceptance criteria for Phase 2

Phase 2 is complete when all of the following are true:

1. The TUI memory panel (opened via `Alt+m`) displays: current turn's injected memories (from manifest), last 24h writes, per-scope counts, and a live search. Real-time updates when new writes occur or new manifests persist. Destructive ops require `y` confirmation.
2. Manifest inspection (`i` key on Injected Now section) opens the full candidate pool for the current turn with per-item score breakdown. Users can navigate to previous turns via `h`.
3. Panel edits (delete, pin, scope-change, text edit) go through `WriterMessage::PanelEdit` variants and never touch `MemoryStore` directly. Verified by a test that wires a mock writer and performs each edit.
4. Panel refresh latency is <10ms per section under normal load. Verified by profiling with 10k memory rows.
5. Claude Code and Codex subprocesses, when spawned by Gaviero, successfully call `memory_search`, `blast_radius`, and `node_doc` MCP tools and receive structured responses. Verified by an integration test that runs each backend against a small task.
6. External MCP memory servers (at minimum `@modelcontextprotocol/server-memory`) are detected on startup and disabled with a user-visible notification offering migration. Verified by seeding a test config and observing the migration.
7. Every MCP tool call is visible as an event in the TUI memory panel. Timing and tool name logged.
8. Graceful degradation: if Codex's MCP support breaks with a current version, Gaviero continues to work (fall back to prompt-time injection only). Verified with a mock Codex that rejects MCP config.
9. No regression in Tier S or Phase 1 of Tier A acceptance criteria.

---

## Anti-patterns to avoid

- **Panel edits bypassing the writer task.** Every edit is a `PanelEdit` `WriterMessage`.
- **Displaying stale manifest data.** Subscribe to `ManifestObserver::on_manifest_persisted`; re-query on event.
- **Synchronous MCP calls blocking the subprocess agent's turn.** MCP tool handlers should be fast (<100ms for `memory_search` on a populated db).
- **Baking Claude Code or Codex specifics into the MCP server.** The server exposes MCP; the config generation for each backend is the point of specialization, isolated in per-backend modules.
- **Exposing memory writes via MCP tool.** All write surface goes through the writer task via transcript or annotations.
- **Logging MCP reads as tool-output memories.** This creates feedback loops. Log to observer events only.
- **Letting the stdio shim be a long-lived unreliable process.** If a shim is needed, it must be robust to Gaviero restart (reconnect to the socket) or transparent (the MCP SDK lets Gaviero speak stdio directly).
- **UI refresh on every observer event without debounce.** 100ms debounce minimum.

---

## Final instruction

Produce the implementation plan per the **Expected output format** section above, covering A4 and A5 in the recommended order (A4 → A5). Split A4 into shippable sub-sections (Injected Now first, then Recently Written, then Scope Summary, then Search). Be specific about MCP SDK choice, per-backend config file shapes, and subprocess-spawn-time hooks for config synthesis. Flag every decision the developer must make before they can start. If any part of the input is ambiguous, call it out in the "Open questions" section of the relevant item rather than guessing.
