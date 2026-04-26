# Tier A Part 1 — Schema, Scope, and Signal

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, call sites, data flow, task ordering, test strategy, acceptance criteria, and open decisions the developer must make. Flag decisions that require human judgment rather than deciding them yourself. Do not produce full Rust code listings — trait signatures, struct skeletons, and pseudo-code snippets are acceptable where they reduce ambiguity.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, ratatui, and the Model Context Protocol (MCP). Do not explain these.

**This document covers Phase 1 of Tier A**, containing three items (A3, A2, A1). The remaining items (A4, A5) are covered in a separate document (**Tier A Part 2 — Surface**). Treat Phase 1 as self-contained: the plan produced here should be implementable without needing Part 2, though ordering and observer callbacks set up in Phase 1 are consumed by Phase 2.

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

### Memory subsystem (current state, assuming Tier S is in place)

- 5-level scope hierarchy: `Global → Workspace → Repo → Module → Run`
- One SQLite WAL file per workspace (`<workspace>/.gaviero/memory.db`) + a separate global file
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- Cascading retrieval with early exit at 0.70 (RRF 0.7 vec / 0.3 fts) — these specifics are a later tier's concern; keep them as-is here
- Composite scoring: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust_multiplier`
- SHA-256 content hash dedup, explicit `WriteScope`
- **Writer task** owns all writes (Tier S2); all subsystems enqueue `WriterMessage` variants
- **Chat auto-inject** in place (Tier S1); default scope set `Workspace ∪ Repo ∪ Module`
- **Per-turn extractor** in place (Tier S3); emits 0–5 candidates per turn via the writer task
- **Retrieval manifest persistence** (Tier S4): every injection produces a persistent `injection_manifests` row; 30-day retention
- `source` and `trust` fields exist on memory records as `"llm_extracted"` / 0.6 for extractor output — but they are not fully plumbed; **A3 in this phase completes this**
- `/remember <text>` is the only user-facing chat write; default scope is likely `Run` (confirm during implementation)

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
11. **All memory writes go through the Tier S2 writer task.** No direct `MemoryStore::store_scoped` calls from outside it.

### Decisions already taken (do not re-litigate)

- **MCP surface is read-only.** Writes never go through MCP.
- **Single-consumer writer task** (built in Tier S). New subsystems in this phase add `WriterMessage` variants or call `WriterHandle::enqueue`.
- **Turn annotations sidecar** (A1 in this phase): LLM emits `<turn_annotations>` JSON block; transcript processor strips it; flags feed writer task.
- **Three-cadence consolidation.** Per-turn (done in S3) + per-session (Tier B5) + sleeptime (Tier B5).
- **No LLM writes.** LLM proposes; writer decides.
- **No graph-RAG / Neo4j / A-MEM memory evolution.**
- **Keep nomic-embed-text-v1.5 for now.**

### What Tier S delivered (assume in place)

- Chat auto-injects memory at prompt assembly time
- Single-consumer writer task with `WriterMessage` enum including `UserRemember`, `TurnComplete`, `SwarmConsolidate`, `InjectionManifest`
- Per-turn extractor producing 0–5 candidates per turn asynchronously
- **Retrieval manifest persistence** (S4): every injection produces a persistent `injection_manifests` row with candidate pool, scores, selections, and exclusion reasons. 30-day retention. CLI introspection via `gaviero-cli memory manifest`.
- `MemoryObserver` trait with `on_write_enqueued`, `on_write_committed`, `on_write_failed`
- `ManifestObserver::on_manifest_persisted(turn_id)` callback
- `AcpObserver::on_memory_injected(summary)` callback
- Settings schema: `memory.chatInjection.*`, `memory.extractor.*`, `memory.manifests.*`

### What Phase 2 (A4, A5) will deliver (not in scope here)

Phase 2 builds the TUI memory panel (A4, consumes the `source`, `trust`, and annotation fields added here) and the read-only MCP server surface for subprocess agents (A5). Nothing in Phase 1 depends on Phase 2, but Phase 2 depends on Phase 1 being complete.

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle (LLM-as-OS terminology): History is immutable raw interaction log (made explicit in Tier C1); Memory is derived, indexed, mutable views used for retrieval (what most of this phase refines); Scratchpad is ephemeral working state (swarm's discovery board; not a concern for chat). Phase 1 of Tier A refines Memory metadata (trust, source), user-facing write semantics (scope correctness), and the high-signal LLM-authored write channel (annotations sidecar). A1's `session_thread` sits at the boundary between scratchpad and memory and is later consumed by Tier B5 session consolidation.

---

## Phase 1 context and rationale

Phase 1 delivers the features that the rest of the tier plan depends on. Each item is either a schema change (cheap now, painful later) or enables a write-side feature used by Phase 2 and later tiers:

- **A3 (trust and source attributes)** — Foundational schema. Every other feature either produces memories (needs to tag them with source/trust) or displays them (needs to show source/trust badges). Cheap now, painful later.
- **A2 (scope correction)** — A common UX complaint ("I said remember and it forgot") traces to scope mismatch, not retrieval failure. Near-zero engineering cost for disproportionate UX gain.
- **A1 (annotations sidecar)** — Richer signal from the LLM. The model has full turn context the extractor doesn't; letting it directly flag durable facts raises precision dramatically. Also produces `session_thread` used by Tier B5.

**Dependencies and ordering within Phase 1:**

- **A3 should land first.** Every other feature writes memory with `source` and `trust`; without A3 these are inexpressible.
- **A2 can land second.** Independent from A3 architecturally but benefits from being able to tag `/remember` writes with `source = "user_remember"`, `trust = 1.0`.
- **A1 should land last in Phase 1.** Needs A3 to tag annotated flags with `source = "llm_annotated"`, `trust = 0.7`; needs the dedup discipline from S3 to dedup annotated flags against extractor output.

**Recommended sequence:** A3 → A2 → A1.

---

## Items in this phase

### A3. Per-memory `trust` and `source` attributes

**Problem.** The Tier S extractor is hardcoded to tag its output with `source = "llm_extracted"` and `trust = 0.6`, but the full schema and downstream consumers do not yet exist. Without a principled trust gradient, LLM-authored memories (extractor, annotations, future consolidation) pollute retrieval as they accumulate.

**Architecture.**

- Schema migration adds two columns to the memory record:
  - `source TEXT NOT NULL` — enum (stored as string for forward-compat): `user_remember | user_panel | llm_annotated | llm_extracted | llm_consolidated | swarm_consolidated | mcp_import | tool_output`
  - `trust REAL NOT NULL DEFAULT 0.6` — float in [0.0, 1.0]
- Update the composite scoring formula to multiply by `trust`:
  - Before: `score = (sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier`
  - After: `score = (sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust`
  - Note: trust is already conceptually in the formula per the existing docs but may not be plumbed; verify and complete.
- Default `trust` values at write time:
  - `user_remember` / `user_panel` → `1.0`
  - `llm_annotated` → `0.7`
  - `llm_extracted` → `0.6`
  - `llm_consolidated` / `swarm_consolidated` → `0.75`
  - `mcp_import` → `0.5` (one-time imports from external MCP memory servers during the disable-MCP migration in Phase 2)
  - `tool_output` → `0.85` (e.g., compiler/test output captured as memory)
- Schema migration is a one-shot: all existing rows get `source = "unknown_legacy"` and `trust = 0.75`. Add a migration version flag in a `_gaviero_meta` table.
- Panel edit operations (Phase 2 A4) can raise trust explicitly when a user verifies an LLM-authored memory.

**Benefits.**

- Lets LLM-generated memories participate in retrieval without drowning user-authored ones. A user note at 1.0 × 0.94 similarity outranks an LLM note at 0.6 × 0.96 similarity for marginal queries.
- Makes trust gradient auditable when something goes wrong ("why did that wrong memory rank so high?").
- Foundational for all downstream work — the panel shows source badges, the sleeptime pass re-scores trust based on usage, the `/forget` command can filter by source.

**Risks.**

- **Schema migration on active `memory.db`.** Write a clean forward migration; test on a fixture db; document rollback (drop columns, restore backup).
- **Another scoring multiplier.** Another tuning knob; more places for regressions. Include A3 in the retrieval regression test suite.
- **Trust set at write time may drift from reality.** An LLM-extracted memory referenced heavily over months clearly has become reliable. Plan (in Tier B5 sleeptime + B6 telemetry) to upgrade trust based on access patterns. Do not implement that here; leave the infrastructure ready.

**Alternatives considered and rejected.**

- *Boolean `is_verified` flag.* Simpler but loses the gradient, which matters at the margins.
- *Separate tables per source.* More invasive, cross-type queries harder.
- *No trust at all, rely on importance.* Conflates "how important is this" with "how reliable is this" — orthogonal concerns.

**Integration notes.**

- All existing write paths need a `source` argument. This is a Rust API change to `MemoryStore::store_scoped` (or wrapping function). Update signatures; rely on the compiler to find call sites.
- Migration is idempotent: check `_gaviero_meta.schema_version` before attempting.
- sqlite-vec virtual table may not need change; additions are to the adjacent metadata table.
- Add `source` and `trust` to the `ScoredMemory` struct returned by searches.
- The S4 manifest already carries `trust_multiplier` per candidate; after A3 this becomes meaningful rather than a placeholder.

---

### A2. `/remember` scope correction and scope-visible UX

**Problem.** The current `/remember <text>` likely defaults to `Run` scope (to be confirmed during implementation — inspect current handler). Run-scoped memories die with the session, producing the reported "I said remember and it forgot" failure mode. Users have no visible indication of what scope was chosen.

**Architecture.**

- **Change default scope of `/remember` from whatever it is today to `Repo`.** This is a behavior change; gate it behind a settings key so power users can revert: `memory.remember.defaultScope = "repo"`.
- Add explicit scope variant commands, all parse through the same `/remember` handler with a scope override:
  - `/remember <text>` → `Repo` (or user's configured default)
  - `/remember-here <text>` → `Run` (session-local)
  - `/remember-module <text>` → `Module` (computed from current file if any, else error)
  - `/remember-workspace <text>` → `Workspace`
  - `/remember-global <text>` → `Global`
- On successful write, confirmation line in chat displays scope badge: `✓ Remembered [Repo] "Chose tokio::sync::Mutex over std::sync..."`
- When the user `/remember`s something that semantically near-duplicates an existing memory at a broader scope, the confirmation notes: `✓ Reinforced existing [Workspace] memory (similarity 0.94)`. This teaches users the scope hierarchy by example.
- Tab-completion in the TUI command-line offers all variants once the user types `/remember`.

Settings schema additions under `memory.remember`:

- `defaultScope: "run" | "module" | "repo" | "workspace" | "global"` (default: `"repo"`)
- `showScopeBadge: bool` (default: `true`)
- `showSimilarityOnReinforce: bool` (default: `true`)

**Benefits.**

- Addresses the most common UX complaint with near-zero engineering cost.
- Visible scope badges train user intuition so future writes self-correct.
- Explicit variants give power users precise control without flag parsing.

**Risks.**

- **Behavior change for existing users.** Announce in release notes; respect `memory.remember.defaultScope` override.
- **"Repo" default may be wrong for users working multi-repo in a workspace.** Settings-configurable default mitigates. Document clearly.
- **Module scope requires current file context.** In chat without a focused file, fall back to `Repo` with a note: `ℹ Module scope requires a focused file; stored at [Repo] instead.`

**Alternatives considered and rejected.**

- *LLM-inferred scope from content.* Rejected — violates Gaviero's existing "never inferred" `WriteScope` invariant.
- *Prompt user for scope on every `/remember`.* Too much friction for a fast-path command.
- *Default to `Workspace`.* Defensible but assumes multi-repo workspaces; single-repo users would over-scope.

**Integration notes.**

- The `/remember` handler enqueues a `UserRemember` `WriterMessage` on the writer task with a `oneshot` ack; TUI displays a spinner until the ack fires, then the badge.
- If the writer task is saturated and the ack times out (>500ms), display `⧖ Queued [Repo] ...` instead of failing.
- Persist the user's scope choice pattern as telemetry input for future "learn preferred scope" features — but do not act on it in this phase.
- The `source` for `/remember` writes is `"user_remember"` with `trust = 1.0` (from A3).

---

### A1. Turn annotations sidecar (structured LLM signal channel)

**Problem.** The Tier S extractor infers what's durable from the transcript alone, often missing signal the LLM itself knows is important. Meanwhile, the LLM has no clean channel to flag "this bit is worth remembering" without polluting user-visible output.

**Architecture.** The system prompt injected into chat turns teaches the LLM to emit a structured sidecar block at the end of every response:

```
<turn_annotations>
{
  "flags": [
    { "type": "decision", "importance": 0.8, "scope": "repo",
      "text": "...", "refs": ["src/foo.rs:L42"] }
  ],
  "session_thread": "investigating worktree cleanup races",
  "open_questions": ["what happens if a swarm agent panics mid-worktree?"]
}
</turn_annotations>
```

The transcript processor (in `acp::client::AcpPipeline` or a thin layer above it):

1. Scans the assistant's final response for the `<turn_annotations>...</turn_annotations>` delimiter pair.
2. Extracts and parses the JSON; on parse failure, logs a warning and proceeds without annotations (never errors the turn).
3. **Strips the block from the response before it reaches the user.** This is critical UX.
4. Attaches parsed `TurnAnnotations` to the `TurnComplete` `WriterMessage`.

Writer task handling of `TurnComplete` with annotations:

- **Two-tier extraction policy.** If annotations are present and `flags` is non-empty, each flag becomes a pre-extracted candidate skipping the extractor LLM call. Flags land with `source = "llm_annotated"`, `trust = 0.7` (from A3). They still pass SHA-256 dedup and near-dup merge (cosine ≥ 0.95) before insert. Hard cap: 5 flags per turn (drop excess with a warning log).
- **Extractor always runs as safety net.** Even when flags are present, the extractor runs on the transcript for signal the LLM didn't self-flag. Extractor output dedupes against just-written flags within a 30-second window (checked by SHA-256 first, then cosine) to avoid double-storage.
- `session_thread` is written to a session-ledger table (separate, lightweight) keyed by `session_id + turn_id`; Tier B5 session consolidator consumes it.
- `open_questions` is stored similarly, not as memories but as session-scoped records for future follow-up mechanisms. Expose them in the TUI (Phase 2 A4) but don't inject them into prompts yet.

**System prompt addition.** A ~150-word section appended to every chat turn's system prompt, teaching the convention, providing an example, and listing the required shape. Include "Always end with `<turn_annotations>{"flags":[]}</turn_annotations>` even if nothing is worth flagging" to discourage omission.

**Benefits.**

- Raises extractor signal quality — the LLM knows more than an external extractor can recover.
- Cuts extractor cost for high-signal turns (annotations present → skip extractor LLM call optionally, or run both and dedup).
- `session_thread` enables clean thematic segmentation for Tier B5 session consolidation.
- Read-only by construction — the LLM proposes but the writer task validates and inserts.

**Risks.**

- **LLM omits the block.** Fall back to extractor silently; never error. Log a `session_omits_annotations_count` metric.
- **LLM over-annotates.** Importance inflation. Mitigate with: hard cap of 5, automatic importance downscaling based on turn duration (turns under 30 seconds cap at 0.7 importance), and Tier B5 sleeptime pass re-scoring.
- **Parse failures.** Use `serde_json::from_str` on the extracted substring; any failure falls through to extractor-only. Record parse failure rate as a metric to drive prompt-iteration if it climbs.
- **Tags in code blocks.** The LLM might emit `<turn_annotations>` inside a markdown fence when explaining how it works. Use a regex that requires the tag to start at the beginning of a line outside any code block; or more robustly, scan only the *last* occurrence in the response (the convention is to place it at end).
- **Prompt-pressure degradation.** On long turns, the LLM may forget the convention. Monitor omission rate over session length; consider re-injecting the convention reminder for turns after token N.

**Alternatives considered and rejected.**

- *Inline `<mem>` markers throughout the response.* User-visible clutter; requires TUI-layer hiding; tags inside code blocks are a parse nightmare.
- *MCP `memory_store` tool.* Reintroduces LLM-as-writer failure mode that Cursor retreated from.
- *Extractor-only (skip annotations).* Works but leaves signal on the table; LLM knows which of its own statements it considers durable.

**Integration notes.**

- The sidecar is transport-independent: works for Claude Code (ACP), Codex, and Ollama identically because it's just text in the response.
- Model-specific prompt caching implications: place the sidecar-instruction portion of the system prompt in the *cached* segment for Anthropic models to avoid re-pricing per turn.
- Versioning: include `"v": 1` in every annotations block. When the schema changes, old-version blocks are either migrated or ignored gracefully.
- Do not emit annotations for prompts that explicitly disable memory (e.g., one-shot `gaviero-cli --no-memory` runs).

---

## Expected output format from the implementation plan

For each of A3, A2, A1 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from Tier S and within this phase); recommended sub-task order.

**3. Affected crates and modules.** Tree-style list of touched files.

**4. New types, traits, and public API.** High-level signatures, struct field lists. Explicitly call out any API changes that affect other subsystems.

**5. Data-flow description.** Plain prose or ASCII: which tasks, which channels, which locks.

**6. Schema / settings / UI changes.** SQL migrations; settings.json additions with types and defaults.

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies.

**8. Test strategy.** Unit, integration, observer, and (for A1) annotation-parse regression tests against fixed transcripts.

**9. Observability.** Tracing spans, metric counters, observer events. Specifically: annotation omission rate, annotation parse failure rate, scope-default override rate, trust-distribution histogram.

**10. Risks and rollback.** Per sub-task: failure modes and reversal steps.

**11. Acceptance criteria.** Per item: measurable conditions testable by a human.

**12. Open questions.** Decisions requiring developer judgment. Examples: whether the annotations sidecar should cache the system-prompt delta for Anthropic prompt caching; what the default scope for `/remember` should be (repo vs workspace); whether extractor should skip entirely when annotations are present or always run as safety net.

---

## Explicit non-goals for Phase 1

Do **not** plan the following here:

- TUI memory panel — Phase 2 A4
- Gaviero-as-MCP-server — Phase 2 A5
- Retrieval algorithm changes (thresholds, RRF weights, cascading behavior) — Tier B
- Embedding model swap — Tier B
- Cross-encoder reranker — Tier B
- Decay policy changes — Tier B
- Session consolidator and sleeptime pass — Tier B (A1 lays groundwork via `session_thread`, but the consolidator itself is later)
- Typed memory stores split — Tier C
- `/forget` command with audit trail — Tier C
- PageRank changes — Tier C
- Typed edges in code graph — Tier C
- KG node-doc schema — Tier D
- Contextual Retrieval — Tier D
- AGENTS.md / CLAUDE.md compat — Tier D

---

## Acceptance criteria for Phase 1

Phase 1 is complete when all of the following are true:

1. Every memory record has non-null `source` and `trust` fields. Legacy rows migrated to sensible defaults (`source = "unknown_legacy"`, `trust = 0.75`). Composite scoring includes `trust` multiplier; verified by regression tests that a user-authored memory at trust 1.0 outranks an LLM-authored memory at trust 0.6 when similarity is comparable.
2. `/remember <text>` without modifier defaults to `Repo` scope (or the user's configured default), shows a scope badge in confirmation, and the memory is retrievable from a fresh chat session. Reinforce-existing detection works: `/remember`ing a near-duplicate of a broader-scope memory produces the reinforcement confirmation rather than a new row.
3. Explicit scope variants (`/remember-here`, `/remember-module`, `/remember-workspace`, `/remember-global`) all work and display correct scope badges.
4. Every chat turn that produces a `<turn_annotations>` JSON block has its `flags` appear in `memory.db` within 5 seconds tagged with `source = "llm_annotated"`, `trust = 0.7`. Verified by prompting the chat with a turn that should produce a durable fact.
5. `<turn_annotations>` blocks are stripped from user-visible response output. Verified by rendering a response to the TUI and grep-checking for the tag.
6. Annotation parse failures never fail the turn; they fall through to extractor-only with a structured log event. Verified by injecting malformed annotation blocks.
7. `session_thread` values are written to a session-ledger table with `session_id + turn_id` keys. Verified by SQL query after a multi-turn session.
8. Extractor dedupes against just-written annotated flags within 30 seconds. Verified by a test where the extractor produces a near-duplicate of an annotated flag and only one row appears in `memory.db`.
9. No regression in Tier S acceptance criteria.
10. The A3 schema migration is idempotent and has been tested against at least one populated production-like `memory.db` fixture.

---

## Anti-patterns to avoid

- **Letting annotations skip dedup.** LLM flags still pass SHA-256 + near-dup before insert.
- **Displaying `<turn_annotations>` blocks to the user.** Strip before render.
- **Inferring scope in `/remember`.** Violates Gaviero's invariant. Default is a configured constant, not a heuristic.
- **Committing trust at write time as if immutable.** Trust can be updated by user action (Phase 2 A4 pin) and later by sleeptime (Tier B5 + B6). Design for it.
- **Failing the turn on annotation parse error.** Always fall through to extractor.
- **Hardcoding trust values inside the writer task per-source.** The mapping from `source` to default `trust` is a table in `gaviero-core::memory::trust_defaults`, not scattered if-else chains.
- **Forgetting the cache-breakpoint for Anthropic models.** The annotation-instruction section of the system prompt must be in the cached segment to avoid re-pricing it every turn.

---

## Final instruction

Produce the implementation plan per the **Expected output format** section above, covering A3, A2, and A1 in the recommended order (A3 → A2 → A1). Be specific about call sites, schema changes, keybindings where applicable, and migration paths. Flag every decision the developer must make before they can start. If any part of the input is ambiguous, call it out in the "Open questions" section of the relevant item rather than guessing.
