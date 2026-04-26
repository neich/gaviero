# Tier S — Foundation: Fix "Starts From Scratch"

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific tier of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, call sites, data flow, task ordering, test strategy, acceptance criteria, and open decisions the developer must make. Flag decisions that require human judgment rather than deciding them yourself. Do not produce full Rust code listings — trait signatures, struct skeletons, and pseudo-code snippets are acceptable where they reduce ambiguity.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, and tree-sitter. Do not explain these.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (a full-screen terminal editor built on ratatui) and `gaviero-cli` (headless runner). It orchestrates upstream coding agents — Claude Code via ACP, Codex via `codex exec` and `codex-app-server`, Ollama via HTTP SSE, and a mock backend — through a provider-agnostic `AgentBackend` trait producing `UnifiedStreamEvent`s. Agent writes to disk flow through a `WriteGatePipeline` for interactive diff review. Multi-agent work is orchestrated via a 6-phase swarm pipeline: validate → execute → merge → verify → cleanup → consolidate.

It is **local-first** and **single-developer-oriented**. No SaaS backend, no cloud persistence, no multi-user concerns. The target user runs it on their own machine against their own code.

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
- `gaviero-dsl` (lib) — `.gaviero` workflow script compiler → `CompiledPlan`
- `tree-sitter-gaviero` — grammar for `.gaviero` files

### Memory subsystem (current state)

- 5-level scope hierarchy: `Global → Workspace → Repo → Module → Run`
- Storage: one SQLite WAL file per workspace (`<workspace>/.gaviero/memory.db`) + a separate global file (`~/.config/gaviero/memory.db`)
- `scope_level` is the sqlite-vec partition key; an FTS5 table provides lexical search
- Retrieval: cascading narrowest→widest with **early exit when best score > 0.70** (RRF hybrid fusing vec 0.7 / fts 0.3)
- Composite scoring: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust_multiplier`
- Write path: embed → SHA-256 content hash → dedup (reinforce / skip-broader / insert) → brief-lock insert into `vec_memories_scoped` + `memories_fts`
- `WriteScope` is explicit (never inferred)
- **Consolidation runs only after swarm execution**: triage (importance ≥ 0.4) → decay (30-day half-life) → cross-scope promotion (3+ module hits → repo, ×1.2 boost). Ordinary chat turns do not consolidate.
- Chat command `/remember <text>` is the only user-facing write
- Provider-agnostic `Embedder` trait wraps `OnnxEmbedder`

### Agent session / prompt construction

`context_planner/` owns bootstrap / delta / replay policy. `PlannerInput → PlannerSelections` (memory, graph, replay, continuity) is produced by `ContextPlanner::plan`. `agent_session::build_turn` lifts `PlannerSelections` into a `Turn`, which the transport layer (`AgentSession::send_turn`) sends to the provider. **Swarm agents use this path with explicit `memory { read_ns }` blocks in their DSL declarations.** Chat interactions with the user go through `acp::client::AcpPipeline::send_prompt`, which enriches prompts before sending but currently does not invoke `ContextPlanner` for memory retrieval.

### Hard constraints (non-negotiable)

1. All agent writes flow through `WriteGatePipeline`; no direct `fs::write` from agent paths.
2. Use `git2` only; never shell out to `git`.
3. Tree-sitter is the source of truth for syntax/indent/highlight.
4. `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`; embedding always runs outside the lock.
5. **No Mutex is held across `await`, tree-sitter parse, or `fs` I/O** — the golden rule.
6. `gaviero-core` contains no UI or CLI types; coupling to TUI/CLI is via observer traits only.
7. TUI has a single `mpsc::UnboundedReceiver<Event>` main loop; no background task mutates `App` directly.
8. `AgentBackend` + `UnifiedStreamEvent` keep provider selection runtime-pluggable via `TierRouter` + `PrivacyScanner`.
9. No plugin system — configuration via `settings.json` cascade (`.gaviero/settings.json` → `.gaviero-workspace` → `~/.config/gaviero/settings.json` → defaults).
10. Memory writes require explicit `WriteScope`; never inferred.

### Decisions already taken (do not re-litigate)

These are foundational decisions the developer has already made for the overall memory overhaul. Your plan must honor them:

- **MCP surface will be read-only.** External MCP memory servers (`@modelcontextprotocol/server-memory`, Mem0 MCP, memory-bank-mcp) will be disabled. Gaviero will eventually expose its own memory to subprocess agents via a read-only MCP interface, but writes never go through MCP.
- **Single-consumer writer task.** All memory writes go through one dedicated tokio task fed by an `mpsc` channel; no subsystem writes to `MemoryStore` directly outside this task.
- **Turn annotations sidecar.** In a later tier, the LLM will emit a structured `<turn_annotations>` JSON block at turn end. Your plan should not depend on it being present, but the extractor (S3) and writer task (S2) should leave room for it without refactoring.
- **Three-cadence consolidation.** Per-turn extractor (this tier) → per-session consolidator (later tier) → weekly/idle sleeptime pass (later tier).
- **No LLM writes.** The LLM proposes; the writer task decides.
- **Keep nomic-embed-text-v1.5 for now.** Embedder swap to `gte-modernbert-base` is a later tier. Do not plan for it here.

---

## Tier S context and rationale

Gaviero's memory system currently has a reported symptom: "every chat starts from scratch." Diagnosis ranked this symptom's causes by prior probability. The top three are:

1. **No auto-inject on chat prompt assembly** — swarm agents retrieve memory via explicit `memory { read_ns }` blocks, but chat has no equivalent. Stored memories are never read on the chat path.
2. **Consolidation only runs post-swarm** — ordinary chat turns produce no memory at all; Run-scope memories written during chat are never triaged upward.
3. **`/remember` default scope may be Run** — notes die with the session (deferred to Tier A).

Tier S resolves causes 1 and 2, which together constitute the bulk of the symptom. Without this tier, every other memory improvement is invisible to the user.

**Conceptual framing.** Gaviero's memory follows the History / Memory / Scratchpad lifecycle familiar from recent LLM-as-OS literature: **History** is immutable raw interaction log (Tier C1 makes this explicit); **Memory** is the derived, indexed, mutable views used for retrieval; **Scratchpad** is ephemeral per-task reasoning state (swarm's discovery board today; not needed for chat). Tier S stands up the core read path (inject) and write path (writer task + extractor) for Memory, plus the per-turn provenance (manifest) that ties every injection decision to its source context.

**Tier S has no dependencies on earlier work.** Everything in Tier A, B, C, and D depends on Tier S being in place.

**Ordering within Tier S:** S2 (writer task) is a hard prerequisite for S3 (consolidation) and S4 (manifest writes). S1 (auto-inject) can land independently but produces the data S4 records. Recommended sequence: S2 → S1 → S4 → S3, because S2 creates the write-path discipline downstream items rely on; S1 makes the system visibly responsive; S4 captures the injection decisions *before* S3 starts producing new memory that needs auditable retrieval; and S3 closes the write-side gap last.

---

## Items in this tier

### S1. Memory auto-injection on chat prompt assembly

**Problem.** The chat path in `acp::client::AcpPipeline::send_prompt` assembles system + conversation + user prompt without invoking memory retrieval. Any memory the user has written (via `/remember` or earlier swarm consolidation) is never surfaced in chat context. The chat agent genuinely has no way to know it exists.

**Architecture.** Insert a deterministic retrieval stage into the chat prompt assembly path. Each chat turn:

1. Extracts a retrieval query from the latest user message (for MVP, use the message text itself; later iterations may pre-process).
2. Invokes `MemoryStore::search_scoped` across a configurable scope set (default: `Workspace ∪ Repo ∪ Module`; exclude `Run` of prior sessions, include `Global` optionally based on config).
3. Fetches top-K candidates (default K=8) under a token budget cap (default 1000 tokens of retrieved content, not including metadata).
4. Formats results into a clearly-demarcated `<project_memory>` block prepended to the prompt, with per-item scope badge, type, and text.
5. Emits a `ChatMemoryInjected` observer event carrying the retrieved set (for future consumption by the TUI memory panel in Tier A4).

The retrieval itself reuses existing `MemoryStore::search_scoped`; **this is an integration task, not a retrieval-algorithm task.** Do not tune RRF weights, thresholds, or scoring here — those are Tier B concerns.

**Benefits.**

- Eliminates the core "starts from scratch" symptom in one commit.
- Makes every subsequent memory feature observable — without it, stored memories are write-only.
- Matches the standard behavior of every production coding agent (Cursor, Claude Code, Cline, Windsurf, Continue).
- Unblocks measurement: you can't measure "does memory help" until it's actually injected.

**Risks.**

- **Context bloat.** Inject cap must be enforced strictly; 1000 tokens of irrelevant memory hurts more than zero. Build the token counter against the same tokenizer the target model uses (conservative under-count is acceptable).
- **Wrong scope mix.** Including `Global` by default could leak cross-project data; excluding `Workspace` misses project conventions. Make scope mix configurable; default should be conservative.
- **Low-precision retrieval surfaces noise.** Can degrade responses below no-memory baseline. Mitigate by (a) budget cap, (b) minimum similarity threshold below which items are dropped (start at 0.3, make configurable), (c) the Tier B reranker later.
- **Coupling risk.** Do not wire `ContextPlanner` into chat if chat doesn't need its full bootstrap/delta/replay machinery — chat has simpler needs. Prefer a narrower `retrieve_for_chat()` helper.

**Alternatives considered and rejected.**

- *On-demand only via MCP tool calls* (let the LLM ask for memory when it needs it). Rejected because the LLM has no reason to ask at turn start when it doesn't suspect memory exists.
- *File-based injection only (CLAUDE.md / AGENTS.md pattern)*. Complementary (Tier D3) but not a substitute for semantic retrieval of accumulated memories.
- *Classifier-gated injection (Self-RAG style)*. Better precision, more moving parts; defer until observability makes the need measurable.

**Integration notes.**

- Observer event `AcpObserver::on_memory_injected(summary)` should fire before the prompt is sent, so the TUI can reflect what was injected. Add it to the `AcpObserver` trait.
- Settings schema gains `memory.chatInjection` block: `enabled: bool` (default true), `scopes: ["workspace","repo","module"]`, `maxItems: u8` (default 8), `tokenBudget: usize` (default 1000), `minSimilarity: f32` (default 0.3).
- `AcpPipeline::send_prompt` is the natural site; do not add retrieval to a layer that doesn't own prompt assembly.
- The injected `<project_memory>` block should be placed immediately after system prompt and before conversation history. Document the exact placement in the plan because it affects prompt-cache behavior for Anthropic models.

---

### S2. Writer task architecture (single-consumer mpsc pattern)

**Problem.** Gaviero currently has multiple potential write paths into `MemoryStore`: the `/remember` command, swarm post-execution consolidation, any future automatic consolidation, the future TUI memory panel edit flows. Each contends for the `tokio::sync::Mutex` around `rusqlite::Connection`. As soon as LLM-invoking consolidation paths are added (S3 and later tiers), holding locks across LLM calls would violate the golden rule. A principled write-path architecture is needed before S3 can be built safely.

**Architecture.** Introduce a single-consumer writer task pattern:

- Define a `WriterMessage` enum in `gaviero-core::memory` with variants for each write origin: `UserRemember { text, scope, type, importance, refs, ack: Option<oneshot::Sender<WriteResult>> }`, `TurnComplete { session_id, turn_id, transcript, annotations: Option<TurnAnnotations> }`, `SwarmConsolidate { swarm_result }`, `PanelEdit { op, ack }`, `Sleeptime` (used in later tier). Leave the enum `#[non_exhaustive]` to allow additions without breaking pattern-matchers.
- A dedicated tokio task `writer_task(rx: mpsc::UnboundedReceiver<WriterMessage>, store: Arc<MemoryStore>, embedder: Arc<dyn Embedder>, llm: Arc<dyn ConsolidationLlm>) -> !` owns all writes. It drains the channel in order, dispatches per variant, and calls `MemoryStore::store_scoped`.
- Synchronous user writes (`UserRemember`, `PanelEdit`) attach a `oneshot::Sender<WriteResult>` the caller awaits with a timeout. Async writes (`TurnComplete`, `SwarmConsolidate`, `Sleeptime`) are fire-and-forget; failures go to a structured log and a metric counter.
- A `WriterHandle` wraps the `mpsc::Sender<WriterMessage>` and is the only way any caller enqueues writes. `MemoryStore` becomes effectively write-only-through-the-task at the API level; keep the direct methods for the task's internal use but remove them from `pub` surface (or mark `pub(crate)`).
- Writer task is spawned once at workspace-open time as part of `Workspace::open`; the `WriterHandle` is exposed on `Workspace` (or a new `WorkspaceServices` struct) for subsystems to obtain.

**Benefits.**

- Collapses all write concurrency into one serialization point. No lock contention, no interleaved races, trivially debuggable.
- Provides natural backpressure via queue depth.
- Makes extractor/consolidator invocations non-blocking for the user-facing event loop.
- Gives one chokepoint to instrument, rate-limit, and audit. Metrics like "writes per scope per source per hour" become trivial.
- Prepares the ground for Tier A's annotations sidecar (just another message variant) and Tier B's sleeptime pass (just another message variant) without further architectural change.

**Risks.**

- **Unbounded queue under burst load.** A swarm with 32 parallel agents all consolidating could produce a burst. Use `mpsc::unbounded_channel` initially for simplicity, but monitor queue depth metric and plan for either a bounded channel with backpressure semantics or a size-based drop-oldest policy on low-priority message variants.
- **Writer task crash loses in-flight messages.** Accept this initially (in-memory queue). If a user reports lost writes, consider persisting pending `UserRemember` messages to a small SQLite queue table as WAL insurance.
- **Ordering between producers can become fragile.** If a user `/remember`s immediately before an extractor would have written the same thing, the extractor's dedup must handle the just-written case. Dedup is per-content-hash already; document the race and rely on it.
- **Synchronous UX.** `UserRemember` with 500ms oneshot timeout is the defensive UX. If writes take longer, the TUI should show "queued" state — do not block the whole TUI event loop on the oneshot.

**Alternatives considered and rejected.**

- *Mutex around `MemoryStore` with disciplined callers.* What currently exists. Violates golden rule as soon as LLM-invoking writes appear.
- *Multiple writer tasks partitioned by scope.* Adds complexity without clear benefit at single-dev scale.
- *External daemon process (`gaviero-memd`).* Right long-term if multi-TUI sharing or crash isolation becomes a priority, but premature. Design the `WriterMessage` enum so that swapping the transport from mpsc to IPC is a transport change, not an architecture change.

**Integration notes.**

- The writer task should not own the `MemoryStore` Arc exclusively — reads continue to hit `MemoryStore` directly through its Arc, bypassing the writer entirely. Only writes go through the task.
- Consider a `trait ConsolidationLlm` now even though S3 is the first consumer. Keep it minimal (`async fn complete(prompt: String) -> Result<String>`), backed by `AgentBackend` selections via `TierRouter`. This keeps the writer task free of backend-specific types.
- The writer task logs every message received with correlation id; this log is the forensic trail when memory goes wrong. Use structured `tracing` fields, not strings.
- Observer trait: add `MemoryObserver` with `on_write_enqueued`, `on_write_committed`, `on_write_failed` for future panel integration.

---

### S3. Per-turn consolidation (the extractor)

**Problem.** After S2 ships, the writer task exists but nothing feeds it from ordinary chat turns. Consolidation still runs only post-swarm. The second cause of "starts from scratch" remains.

**Architecture.** After every completed chat turn (or every N turns; default N=1), the writer task handles a `TurnComplete` message by:

1. Reading the turn transcript (user message + assistant response + any tool outputs).
2. Invoking a medium-tier LLM via `ConsolidationLlm::complete` with the extractor prompt (see below).
3. Parsing the returned JSON into 0–5 `ExtractedMemory` candidates, each with `type`, `scope_hint`, `text` (≤280 chars), `importance` (0.0–1.0), and `refs`.
4. For each candidate: run SHA-256 dedup against `same-scope + same-type`; if cosine ≥ 0.95 against an existing memory of same scope+type, treat as merge (reinforce importance, update `last_seen`); otherwise insert via `store_scoped` with `source = "llm_extracted"` and `trust = 0.6`.
5. Emit `MemoryObserver::on_write_committed` per inserted item.
6. On LLM unavailability or JSON parse failure: fall back to writing a single `Run`-scope raw record with the user message as text and default importance 0.4. **Never lose the turn.**

The extractor runs **asynchronously inside the writer task** — the chat event loop enqueues `TurnComplete` and returns immediately. The user's next input is never blocked by extraction.

**The extractor prompt.** Version-pinned, in a `const &str` in the memory module. Produces strict JSON only. Rubric:

- `type ∈ { decision, lesson, error, convention, preference, gotcha, invariant }`
- Importance rubric: 0.9+ = architectural; 0.6–0.9 = module-level; 0.3–0.6 = local; below 0.3 = do not emit
- Hard cap: 0–5 extractions per turn
- Do NOT extract: generic programming knowledge, facts derivable from grep/tree-sitter, restatements of user request, tentative plans, assistant intent (only outcomes)
- Output `{"extractions": []}` when nothing is durable

The full prompt text is in the research report the developer has access to; preserve it verbatim as the v1 version. Store the prompt version on each extracted memory for future migration.

**Benefits.**

- Closes the write gap: chat turns now produce memory continuously.
- Token cost is trivial: ~1500 input + ~500 output per call at Haiku/4o-mini/DeepSeek tier → sub-cent per turn.
- Produces structured, typed memories (not free-text) that compose cleanly with scope hierarchy and the later sleeptime pass.
- Graceful degradation: on LLM failure, still writes a minimal record so the turn isn't lost.

**Risks.**

- **Over-extraction (noise).** Memory sprawl pollutes retrieval. Mitigate via: (a) hard cap of 5 per turn, (b) minimum importance threshold of 0.3, (c) dedup against existing, (d) the Tier B sleeptime pass will do near-dup merge later.
- **Under-extraction (silent failure).** Hard to detect without telemetry. Mitigate with per-turn counter of extractions emitted; alert on persistent zero-extraction sessions.
- **LLM unavailability.** Network flake during extraction is invisible to the user. Fallback path is mandatory.
- **Prompt brittleness.** Pin the prompt version on each memory. When the prompt changes, extracted memories retain provenance.
- **Extractor becomes a single quality bottleneck.** All LLM-authored memory quality hinges on this one prompt + model choice. Build tests that lock down extractor output format for a fixed set of input turns (regression tests).

**Alternatives considered and rejected.**

- *Keyword/heuristic extraction (no LLM).* Too brittle for code discourse; misses most real signal.
- *Fully manual (`/remember` only).* Caps quality at user discipline; the primary complaint is that users don't `/remember` enough.
- *Self-refine loop on the extractor output.* Adds cost without evidence of gain. Self-Refine's own data shows second passes are usually vacuous without external signal.

**Integration notes.**

- `ConsolidationLlm` impl should route via `TierRouter` at the "cheap" or "mechanical" tier. Default model: whatever the user has configured for that tier.
- Ack timeout on `TurnComplete` is irrelevant because it's fire-and-forget. But the writer must log the end-to-end latency per turn (parse time, LLM call time, dedup time, insert time) for later perf work.
- Dedup uses the existing `OnnxEmbedder`. Embed must happen outside any lock held by the writer; follow the existing pattern in `store_scoped`.
- Run scope is the default write scope for extracted items unless the extractor specifies otherwise in `scope_hint`. Scope promotion is a later-tier concern; do not promote from the extractor itself.
- Add a `memory.extractor` settings block: `enabled`, `model` (tier reference), `everyNTurns` (default 1), `promptVersion` (readonly, informational), `maxExtractionsPerTurn`.

---

### S4. Retrieval manifest persistence

**Problem.** S1 computes an injection decision on every chat turn (which memories, with which scores, under which budget). S3 writes new memories based on turn content. But there is no persistent record tying "the response the LLM produced" to "the injection decisions that led to it." When retrieval misbehaves — a relevant memory exists but wasn't injected, or an irrelevant one was — there is no forensic trail. Post-hoc eval ("what fraction of Tier 1 test queries had the right memory in the candidate pool but ranked below cutoff?") is impossible because the candidate pool is not recorded.

This gap also prevents a category of quality work downstream: Tier B's eval harness, Tier B5's session consolidator, and Tier B6's retrieval-use telemetry (to be added in Tier B) all benefit from being able to read past injection decisions rather than recomputing them.

**Architecture.** Every chat turn's injection produces a **retrieval manifest** — a structured, persistent record of what happened during retrieval. Written by the writer task as a `WriterMessage::InjectionManifest` variant after S1 completes the injection decision.

Manifest schema (stored in a new table `injection_manifests` in the workspace `memory.db`):

- `turn_id` (links to the turn being retrieved-for)
- `session_id`
- `query_text` (the user message that triggered injection)
- `query_embedding_hash` (SHA of the query vector for reproducibility)
- `candidate_pool` — array of `{memory_id, scope, type, raw_similarity, composite_score, scope_multiplier, trust_multiplier, recency_contribution, selected: bool, exclusion_reason?}` — typically 20–100 entries
- `selected_ids` — convenience list of memory IDs actually injected
- `token_budget_used` / `token_budget_limit`
- `scoring_formula_version` (so you can re-evaluate old manifests after scoring changes)
- `embedder_name` (from the Tier C5 pluggable traits; critical for reproducibility across embedder upgrades)
- `created_at`

**Retention.** Manifests retain for 30 days by default (configurable); pruned by the sleeptime pass (Tier B5). The point isn't long-term archive — it's a rolling window sufficient for eval, debugging, and feedback to the B6 telemetry.

**Writer-task flow.**

1. S1's chat injection computes the manifest as part of its normal work (it already has all the scores).
2. Fires `WriterMessage::InjectionManifest { manifest }` to the writer task.
3. Writer task writes it to `injection_manifests` table. Separate from `memories` table — manifests are not embedded, not semantically searchable, purely a log.
4. Emits `ManifestObserver::on_manifest_persisted(turn_id)` for panel display.

**Panel integration (Tier A4 later).** The memory panel's "Injected Now" section becomes backed by the *current turn's manifest* rather than an in-memory observer snapshot — same data, but now persisted and navigable. A new "Inspect manifest" action opens the full candidate pool with per-item score breakdown.

**Benefits.**

- Automated retrieval debugging. Tier 1 smoke-test failures automatically diagnose themselves ("the right memory was ranked 12th with composite 0.41; threshold was 0.60").
- Unblocks Tier B6 (retrieval-use telemetry) — the telemetry reads yesterday's manifests and correlates which selections were actually used in responses.
- Unblocks a class of retrospective A/B: you can re-score an old manifest's candidate pool under a new scoring formula without re-running the LLM turn.
- Provenance for the Tier C2 audit story — if a memory was deleted or demoted by sleeptime, the manifests show its injection history.
- Near-zero cost: one row per turn, ~2 KB JSON, serialized from in-memory scoring state.

**Risks.**

- **Storage growth.** 2 KB × 50 turns/day × 30-day retention = ~3 MB per active workspace. Trivial. But without retention enforcement it compounds; make sure the sleeptime prune actually runs.
- **Schema lock-in for candidate-pool shape.** The pool entry shape is load-bearing for downstream consumers; versioning via `scoring_formula_version` handles evolution but requires care when scoring changes.
- **Manifest write failure should not fail the turn.** Writer-task's async discipline already gives us this; make explicit in acceptance criteria.
- **PII in query text.** Query text may include user paths, credentials pasted into chat. Same trust boundary as memories themselves (local-first workspace DB), but call out in docs. Do not sync manifests anywhere.

**Alternatives considered and rejected.**

- *Log-only (tracing spans, no DB table).* Adequate for live debugging but non-queryable and not consumable by later tiers. B6 telemetry would be harder.
- *Embed manifests into the memory record itself.* Conflates data (what was retrieved) with metadata (how); breaks clean separation. Also bloats the memory table.
- *Compute on demand (re-run retrieval with historical embedder/formula).* Requires embedder-version-preservation infrastructure; much more complex than just writing the manifest once.

**Integration notes.**

- Settings: `memory.manifests.enabled: bool` (default `true`), `memory.manifests.retentionDays: u32` (default 30), `memory.manifests.captureCandidatePool: bool` (default `true`; turning off reduces storage and captures selected-only).
- Table should be outside the sqlite-vec virtual-table world (no vectors); plain SQLite is sufficient.
- Add a CLI introspection command: `gaviero-cli memory manifest --turn <id>` prints a human-readable breakdown; `gaviero-cli memory manifest --last N` shows the last N.
- The swarm path also goes through S1's retrieval code in the future; manifest capture should be symmetric (chat and swarm both produce manifests; distinguish via a `source_channel` field).
- S4 depends on S1 (must know what was retrieved) and S2 (writer task is the persistence path). It does not depend on S3.

---

## Expected output format from the implementation plan

For each of S1, S2, S3, S4, the plan should contain:

**1. Summary.** 3–5 sentences stating the goal, the user-visible effect, and the non-goals.

**2. Dependencies and ordering.** What must be in place before starting; recommended order across sub-tasks; any blockers from outside this tier.

**3. Affected crates and modules.** Tree-style list of which files in which crates get new code, modified code, or are touched for wiring only.

**4. New types, traits, and public API.** High-level signatures (function names, trait names, struct field lists) — not full code. If a public API changes, note the migration.

**5. Data-flow diagram.** ASCII or plain-prose description of the new flow, including which tasks run on which tokio runtimes, which channels carry which messages, which locks are held where.

**6. Schema / settings changes.** SQL migrations if any; settings.json schema additions with types and defaults.

**7. Task breakdown.** Ordered list of sub-tasks, each with: ID, one-line description, estimated size (S = ≤1 day, M = 2–4 days, L = >4 days), dependencies on other sub-tasks.

**8. Test strategy.** Per sub-task: which unit tests, integration tests, and observer-based tests are needed. For S3 specifically: extractor regression tests against fixed transcripts.

**9. Observability.** Tracing spans and metric counters to add. Specifically for this tier, call out: chat-injection hit/miss, writer-queue depth, extractor success/failure rates, extractor latency.

**10. Risks and rollback.** Per sub-task: what could fail, and how to revert without data loss.

**11. Acceptance criteria.** Per item: measurable conditions for "done." Each criterion should be testable by a human running the TUI or CLI.

**12. Open questions.** Decisions the developer must resolve before or during implementation. Examples for this tier: default scope set for chat injection; whether to run the extractor every turn or every N turns; which model tier the extractor uses; whether to use bounded or unbounded channel for the writer queue.

---

## Explicit non-goals for Tier S

Do **not** plan the following in this tier (they are later tiers):

- Scope correction for `/remember` (Tier A2)
- Per-memory trust/source attributes (Tier A3)
- TUI memory panel (Tier A4)
- Gaviero-as-MCP-server (Tier A5)
- Turn annotations sidecar parsing (Tier A1 — but leave the door open)
- Embedder upgrade (Tier B1)
- Cross-encoder reranker (Tier B2)
- Retrieval algorithm changes like removing 0.70 early-exit (Tier B3)
- Decay policy changes (Tier B4)
- Session consolidator or sleeptime pass (Tier B5)
- Typed memory stores split (Tier C1)
- `/forget` command (Tier C2)
- PageRank changes (Tier C3)
- Typed edges (Tier C4)
- Pluggable embedder/reranker traits (Tier C5)
- KG node-doc schema (Tier D1)
- Contextual Retrieval (Tier D2)
- AGENTS.md / CLAUDE.md compat (Tier D3)

When in doubt about whether something belongs, ask whether it is needed for the "starts from scratch" symptom to visibly resolve. If not, defer.

---

## Acceptance criteria for the overall tier

Tier S is complete when all of the following are true:

1. Starting a fresh chat session in Gaviero and asking a question related to content of prior chats on the same project produces a response that references or builds on the prior content. (Baseline: today, it does not.)
2. After a chat turn completes, a new memory record appears in `memory.db` within 5 seconds (extractor async latency). Verified by querying SQLite directly or via `gaviero-cli memory list --since 1min`.
3. All writes to `memory.db` (from any path — `/remember`, extractor, swarm consolidation, manifests) are observable as `MemoryObserver::on_write_committed` events. Verified by a test that wires a mock observer and performs each write type.
4. Every chat injection produces a persisted `injection_manifests` row within 2 seconds of the turn starting. Running `gaviero-cli memory manifest --last 1` shows the full candidate pool with per-item score breakdown. Verified against a seeded DB with known-good queries.
5. No Mutex is held across any `await` in the memory-write path. Verified by `cargo clippy` with a strict config and by a code review checklist.
6. Turning off `memory.chatInjection.enabled = false` in settings causes Gaviero to behave exactly as today (no injection, no manifests). Regression safety.
7. Killing the LLM provider mid-extractor-call does not lose the turn; the fallback path writes a minimal record and logs the failure. Verified by injecting a mock LLM that always errors.
8. Manifest-write failure does not fail the turn. Verified by injecting a mock writer task that errors on `InjectionManifest` messages; the turn still completes and responds normally.
9. `memory.db` schema changes are forward-compatible or carry an upgrade migration that is tested against at least one populated production-like `memory.db` fixture.

---

## Anti-patterns to avoid

- **Blocking the user's next input on extraction.** The extractor is async. Period.
- **Letting the LLM write directly to `MemoryStore`.** All writes go through the writer task. The extractor is inside the writer task; the LLM output is parsed data, not a direct call.
- **Skipping dedup.** Even extractor-generated memories must pass SHA-256 content check and semantic near-dup check before insert.
- **Tight coupling to a specific LLM provider.** `ConsolidationLlm` is an abstraction over `AgentBackend`. Do not hardcode Anthropic or OpenAI in the extractor path.
- **Silent failures.** Every failure path emits a structured log event with enough context to debug. "Extractor failed" with no correlation id is forbidden.
- **Overly clever prompt engineering.** The extractor prompt is boring, strict JSON, explicit rubric. If the plan proposes a chain-of-thought extractor or a self-critique loop, that is out of scope.
- **Re-inventing retrieval.** S1 reuses `MemoryStore::search_scoped`. No new retrieval algorithm.
- **Losing a turn because a manifest write failed.** Manifest writes are fire-and-forget; turn completion never depends on them.

---

## Final instruction

Produce the implementation plan per the **Expected output format** section above, covering S1, S2, S3, and S4 in the recommended order (S2 → S1 → S4 → S3). Be specific. Flag every decision the developer must make before they can start. If any part of the input is ambiguous, call it out in the "Open questions" section of the relevant item rather than guessing.
