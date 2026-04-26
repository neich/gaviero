# Tier B Part 2 — Consolidation and Feedback Loop

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, data flow, migrations, evaluation strategy, task ordering, and acceptance criteria. Flag decisions that require human judgment. Do not produce full Rust code listings.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, and the relevant retrieval/embedding literature. Do not explain these.

**This document covers Phase 2 of Tier B**, containing two items (B5, B6) that together complete the three-cadence consolidation system and add a cheap feedback loop from retrieval-use back into memory trust. Phase 1 of Tier B (B4 decay, B1 embedder upgrade, B2 reranker, B3 merged multi-scope) is covered in a separate document and is a **prerequisite** for full value — especially the B1 embedder upgrade, which is reused by B6 for response embedding. Treat Phase 2 as self-contained for planning, but do not start implementation until Phase 1 has landed.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (ratatui TUI editor) and `gaviero-cli` (headless runner). It orchestrates Claude Code (ACP), Codex, Ollama, and mock backends through a provider-agnostic `AgentBackend` trait. Agent writes flow through a `WriteGatePipeline`. Swarm orchestration runs a 6-phase pipeline.

Local-first, single-developer-oriented.

### Stack

- Rust 2024, tokio, petgraph, ropey, git2, ratatui 0.30
- SQLite (WAL mode) + sqlite-vec
- ONNX Runtime (`ort` crate) with **`gte-modernbert-base` embedder** (post-B1) — B6 reuses this for response embedding
- **Cross-encoder reranker** enabled in the retrieval pipeline (post-B2)
- tree-sitter 0.25, 16-language registry

### Crate layout

- `gaviero-core` (lib) — all runtime, no UI
- `gaviero-tui`, `gaviero-cli`, `gaviero-dsl`, `tree-sitter-gaviero`

### Memory subsystem state (assuming Tier S, Tier A, and Tier B Phase 1 are in place)

- 5-level scope: `Global → Workspace → Repo → Module → Run`
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- **Merged multi-scope retrieval** (B3) — no early-exit; scope bias via multipliers (defaults: Run 1.10, Module 1.05, Repo 1.00, Workspace 0.95, Global 0.85)
- **Cross-encoder reranker** `gte-reranker-modernbert-base` (B2); blended 0.6 rerank / 0.4 composite by default
- **`gte-modernbert-base` embedder** (B1)
- Composite scoring: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust`, with reranker blend
- **Decay floor at 0.35; type-based exemptions** for decision/convention/invariant/preference (B4)
- SHA-256 + semantic near-dup dedup on writes
- **Writer task owns all writes** (Tier S2)
- **Chat auto-inject** at prompt assembly (Tier S1)
- **Retrieval manifest persistence** — every injection writes an `injection_manifests` row (Tier S4), now including `rerank_score` per candidate. **B6 consumes manifests directly.**
- **Per-turn extractor** producing structured memories (Tier S3)
- **Turn annotations sidecar** providing high-signal LLM-flagged candidates with `session_thread` (Tier A1). **B5 consumes `session_thread` for thematic segmentation.**
- **`/remember` scope-corrected and visible** (Tier A2)
- **`source` and `trust` attributes** on all records (Tier A3). **B5 and B6 update trust.**
- **TUI memory panel** for inspection and curation (Tier A4)
- **Gaviero-as-MCP-server** exposing read-only tools (Tier A5)
- `Embedder` and `Reranker` traits with ONNX impls

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle. Phase 2 of Tier B is where the Memory layer becomes actively curated over time, not just written once: B5 adds the session and sleeptime cadences that segment, merge, supersede, and promote memories; B6 feeds a usage signal back into trust scoring so memories that prove themselves get protected and memories that don't fade. Nothing in this phase touches History or Scratchpad directly.

### Agent session / prompt construction

`agent_session::build_turn` produces a `Turn` from `PlannerSelections`; chat goes through `acp::client::AcpPipeline::send_prompt`.

### Hard constraints (non-negotiable)

1. All agent writes flow through `WriteGatePipeline`.
2. `git2` only.
3. Tree-sitter for syntax work.
4. `MemoryStore` behind `tokio::sync::Mutex`; embedding outside lock.
5. **No Mutex across `await`, parse, or `fs` I/O.**
6. `gaviero-core` has no UI/CLI types.
7. Single TUI `mpsc::UnboundedReceiver<Event>` main loop.
8. Provider-agnostic `AgentBackend`.
9. Config via `settings.json` cascade.
10. Memory writes require explicit `WriteScope`.
11. **All memory writes go through the Tier S2 writer task.** B5 consolidation operations and B6 telemetry writes are no exception.
12. **MCP surface is read-only.**

### Decisions already taken (do not re-litigate)

- **Writer task, annotations sidecar, read-only MCP** — all architectural.
- **Three-cadence consolidation.** Per-turn in place (S3); session + sleeptime in this phase (B5).
- **No LLM writes.** Consolidator proposes; writer task applies.
- **No graph-RAG / Neo4j / A-MEM memory evolution.**
- **No LLM-based response evaluator.** B6 uses cheap cosine + substring, explicitly not an LLM judge (reviewed against Xu et al. 2025's Context Evaluator proposal; cost not justified).

### Evaluation infrastructure

The three-tier eval harness from Phase 1 is assumed in place:

- **Tier 1 — Retrieval smoke test.** 30–50 pinned pairs; recall@1/5/10 + MRR; runs in <1min.
- **Tier 2 — Code-specific memory eval.** 20–30 multi-session scenarios; LLM-as-judge with manual cross-check.
- **Tier 3 — SWE-bench subset.** 20 issues; A/B memory on/off; weekly.

Phase 2 must gate on Tier 1 non-regression. B5 additionally requires Tier 2 non-regression after session-consolidator enabled, and a dry-run pass on a populated `memory.db` before enabling sleeptime merge/promote in production.

---

## Phase 2 context and rationale

- **B5 (session consolidator + sleeptime pass).** Completes the three-cadence system. Session segments by `session_thread` (from Tier A1); sleeptime does bulk hygiene (decay sweep, near-dup merge, cross-scope promotion, trust re-scoring, KG refresh when Tier D1 lands). Largest item in Phase 2.
- **B6 (retrieval-use telemetry).** Post-turn cheap check: did the response actually use the injected memories? Produces a per-memory utilization rate that feeds B5's sleeptime trust re-scoring. Lightweight (no LLM call); reads S4 manifests as input.

**Dependencies and ordering within Phase 2:**

- **B5 ships first.** B6 feeds the sleeptime trust-rescoring hook; that hook must exist before B6 produces data.
- **B6 depends on S4 (manifests)** for the injected-memory set and on the B1 embedder (post-Phase-1) for cheap response embedding.

**Recommended sequence:** B5 → B6.

---

## Items in this phase

### B5. Per-session consolidator and sleeptime pass

**Problem.** Tier S3 gave us per-turn extraction. But two cadences are missing:

- **Per-session consolidation** — at chat close (or extended idle), analyze the session's extracted memories against existing memory, produce a session summary, and emit dedup/merge/promote operations. Without this, session-level coherence is never captured; each turn's extractions live as isolated fragments.
- **Sleeptime/weekly pass** — expensive operations that don't fit in foreground cost budgets: decay sweep, cross-session near-duplicate merge, trust re-scoring based on usage patterns, cross-scope promotion (Run→Module→Repo based on co-occurrence), KG node-doc refresh if Tier D1 is active.

**Architecture.**

**Per-session consolidator.**

- Trigger: chat session closes OR idle for configurable timeout (default 90 seconds of no user input AND no in-flight extraction) OR explicit `/consolidate-session` command.
- Input:
  - Recent `TurnComplete` memories from this session (query `memory.db` by `session_id`).
  - Transcript of the last N turns (trimmed if long).
  - Existing memories that might be related (retrieved by similarity against the session's new items, using the post-Phase-1 retrieval pipeline).
- Execution: dispatched to the writer task as `WriterMessage::SessionConsolidate { session_id }`. Writer task invokes a medium-tier LLM via `ConsolidationLlm` with the consolidator prompt (version-pinned in `gaviero-core`; preserve verbatim as v1).
- Output: strict JSON with:
  - `session_summary` — AI-oriented briefing (≤400 tokens)
  - `operations` — per-item `ADD | MERGE | SUPERSEDE | DROP` decisions
  - `promotions` — suggested scope-level moves
- Writer task applies operations: MERGE updates an existing memory's text and importance; SUPERSEDE marks an old memory as superseded (soft-delete with reference; feeds into Tier C2 `/forget` audit trail when that lands); DROP deletes the candidate before it's persisted beyond Run scope.
- Session summary stored as a `summary`-type memory at the session's working scope (usually Repo or Workspace, determined by `session_thread` context). This row's `memory_kind` will be `summary` once Tier C1 lands; until then, it's stored as a distinct memory type with `type = "session_summary"`.

**Sleeptime pass.**

- Trigger: machine idle >10 minutes AND last-sleeptime-run >24 hours, OR weekly cron (whichever first), OR explicit CLI command `gaviero-cli memory sleep`.
- Dispatched as `WriterMessage::Sleeptime`.
- Operations (in order, all inside the writer task):
  1. **Decay sweep.** Iterate memories, compute current recency (respecting B4 floor and exemptions), flag unretrievably-low candidates for user review (do not delete automatically).
  2. **Near-duplicate merge.** For each scope, find pairs with cosine ≥ 0.92 and same type, merge (keep higher-trust, update text if user edited, combine refs). This is bulk dedup beyond what the per-write dedup catches (different phrasings of the same fact written on different days).
  3. **Cross-scope promotion.** The existing "3+ module hits → repo" logic runs. Extended to: items of type=`decision|convention|invariant` with any cross-module evidence promote after 1 hit (lower threshold for high-value types).
  4. **Trust re-scoring.** LLM-authored memories accessed (injected + retrieved in MCP calls) >5 times without user deletion upgrade trust by +0.1 (capped at 0.9 for LLM-origin; user_remember is already 1.0). **B6 telemetry data is consumed here once Phase 2 B6 lands; until then, re-scoring uses raw injection counts from manifests.**
  5. **KG node-doc refresh.** (Only if Tier D1 is active — skip if not.) Regenerate doc fields for modules whose `public_api` has changed since last regeneration.
- Emits `SleeptimeObserver::on_operation_complete` events for the panel.
- Provides a dry-run mode (`gaviero-cli memory sleep --dry-run`) that logs what would change without writing.

**Benefits.**

- Session consolidation captures thematic coherence that per-turn extraction misses.
- Sleeptime provides the bulk hygiene that per-turn can't afford.
- Three-cadence system is the pattern validated across Letta, Mem0, and cognitive-science-inspired systems.
- Dry-run mode makes the destructive operations auditable.

**Risks.**

- **Three consolidation paths means three places for bugs.** Extensive integration tests. Each consolidation variant has its own regression fixture.
- **Sleeptime merge/promote can destroy user-added memory.** Mitigate with: source-aware merging (`user_remember` always takes precedence over `llm_*` when merging); audit log of every sleeptime operation (becomes the Tier C2 `/forget` audit trail); easy revert via TUI panel.
- **Idleness detection is ambiguous.** Start with simple wall-clock idle; refine based on observation.
- **Session close detection.** When does a session "close"? Options: TUI shutdown, explicit `/end-session`, no-activity timeout. Support multiple triggers; pick a conservative default (no-activity 90s).
- **LLM unavailability during consolidation.** Consolidation is deferrable — queue the work, retry on next trigger. Never block the user.
- **First-run on populated `memory.db` surprises.** A user with 6 months of Gaviero use running sleeptime for the first time might see dozens of merges. Require explicit first-run confirmation: "First sleeptime pass detected; N near-duplicates will be merged. Continue? (dry-run recommended)".

**Alternatives considered and rejected.**

- *Per-turn only.* Leaves thematic consolidation and periodic hygiene on the table.
- *Per-session only.* Loses in-session recall and timely hygiene.
- *Synchronous consolidation at fixed intervals.* Blocks user input.
- *Single unified "sleep" that does both session and sleeptime work.* Conflates latency budgets and failure modes; keep distinct.

**Integration notes.**

- `session_thread` (from Tier A1 annotations) feeds the consolidator — use it to segment long sessions thematically before summarization.
- Sleeptime pass is idempotent: running it twice with no intervening activity is a no-op.
- Observer events flow to the panel for real-time display during sleeptime; don't block sleeptime on observer delivery.
- Session summaries are retrieved as part of normal memory search once Tier C1 lands (where `memory_kind = "summary"` is topic-matched); until then, they appear as low-injection-weight summary-type rows.
- Settings:
  - `memory.session.consolidateOnClose: bool` (default `true`)
  - `memory.session.idleTimeoutSec: usize` (default 90)
  - `memory.sleeptime.enabled: bool` (default `true`)
  - `memory.sleeptime.minIdleMinutes: usize` (default 10)
  - `memory.sleeptime.weeklyForceRun: bool` (default `true`)
  - `memory.sleeptime.nearDupThreshold: f32` (default 0.92)
  - `memory.sleeptime.firstRunRequireConfirm: bool` (default `true`)

---

### B6. Retrieval-use telemetry

**Problem.** Once manifests (S4) record every injection decision, a question becomes answerable that wasn't before: *were the injected memories actually useful for the response?* A memory ranked in the top 5 and injected into context may still be ignored by the model. If a memory is consistently injected but never used, it's either bad (stop trusting it) or redundant (consolidate it away). If a memory is consistently used, it's load-bearing (trust it more, protect it from decay).

Today we have importance (how the write said to weight it), trust (how reliable the source is), and recency (how fresh it is). We have no signal for *usefulness* — did retrieval actually help? B6 closes that gap cheaply, without an LLM call.

**Architecture.** After each chat turn completes, a lightweight telemetry pass runs inside the writer task (triggered as part of `TurnComplete` handling, after the extractor has already consumed the transcript):

1. Read the S4 manifest for this turn to get the set of injected memory IDs and their texts.
2. Compute cosine similarity between the assistant response embedding and each injected memory's embedding. Embedding reuses the B1 `Embedder` (`gte-modernbert-base`).
3. Classify each injected memory for this turn as:
   - **Used**: cosine ≥ 0.55 (configurable), OR the response contains a substring match of ≥ 8 consecutive tokens from the memory text.
   - **Partial**: 0.35 ≤ cosine < 0.55.
   - **Unused**: cosine < 0.35.
4. Write a `retrieval_use` row per injected memory per turn:
   ```
   (memory_id, turn_id, injected_rank, classification, cosine_to_response, created_at)
   ```
5. Emit `TelemetryObserver::on_use_classified(turn_id, counts)` for panel display.

**Utilization aggregation.** A `memory_utilization` materialized view (or on-demand computed) gives per-memory:
- `times_injected`, `times_used`, `times_partial`, `times_unused`
- `utilization_rate = times_used / max(1, times_injected)`
- `last_used_at`

**Feedback into B5 sleeptime pass.** Sleeptime trust re-scoring (from B5, step 4) gets a new input: utilization rate.

- Memories with `utilization_rate > 0.6` over ≥5 injections earn +0.05 trust per sleeptime pass (capped at 0.9 for LLM-origin sources; `user_remember` already at 1.0).
- Memories with `utilization_rate < 0.1` over ≥5 injections lose −0.05 trust per sleeptime pass (floor: 0.2).
- Memories with `times_injected = 0` after 30 days (never retrieved at all) are flagged for user review in the panel ("consider pruning?") but not auto-deleted.

**Panel integration (Tier A4 already in place).** The memory panel gains a small utilization indicator per memory row: `↑ 0.72` (used 72% of the time) or `⟂ 0.08` (almost never useful). The manifest-inspection view (from A4) shows per-item classification for the current turn.

**Benefits.**

- Closes the feedback loop on retrieval quality without any additional LLM cost.
- Provides the first objective signal to drive automated memory hygiene (trust re-scoring, flag-for-pruning) — as opposed to write-time heuristics which are fixed forever.
- Surfaces silent retrieval failures that no other mechanism would catch: a memory that's always injected but never used would otherwise live forever at its original trust.
- Becomes an eval signal: retrieval-use rate aggregated across Tier 1 smoke-test turns is a gross proxy for retrieval-precision health.

**Risks.**

- **Cosine is an imperfect signal.** The LLM may reason *around* a memory without mirroring it lexically or semantically. A response like "I checked the pattern and it's fine" is useful but has low cosine to the invariant-memory it consulted. Mitigate: combine cosine with substring match (already done); treat thresholds as soft signals that feed gradual trust re-scoring, not hard deletions. Never auto-delete based on this.
- **Over-active re-scoring destabilizes trust.** If every sleeptime adjusts trust up/down on every memory, trust becomes high-variance. Mitigate: min-sample threshold (≥5 injections) and small per-pass delta (±0.05).
- **Embedding the response costs.** Each turn runs one extra embedder call on the response text. At gte-modernbert speeds, that's ~50ms; acceptable. Cache if the response is very long.
- **Correlation, not causation.** A memory may be "used" because it happened to be topically adjacent, not because it was causally helpful. Accept; the signal is still better than no signal.
- **Privacy/noise on silly chat.** Casual turns ("hi", "thanks") inject memories that get classified as Unused because the response is unrelated. Mitigate: skip telemetry for turns under a token threshold (e.g., 20 tokens of user input).

**Alternatives considered and rejected.**

- *LLM-based usage judgment* ("did the response actually use memory X?"). Doubles per-turn LLM cost; not justified. The Xu et al. 2025 paper proposes a Context Evaluator that does this kind of judgment; keeping it lightweight and cheap here.
- *Token-level attention attribution.* Some model providers expose attention weights. Not portable across backends; tool-coupling.
- *Ignore utilization; rely on write-time importance forever.* Leaves the signal on the table and makes trust stale.
- *Auto-delete low-utilization memories.* Too aggressive; user-visible surprise. Flag for review, don't delete.

**Integration notes.**

- Runs in the writer task, same as other post-turn work; does not block the user.
- The `retrieval_use` table is log-like: append-only, retained 90 days, pruned by sleeptime. The aggregated view is the long-lived artifact.
- Settings:
  - `memory.telemetry.enabled: bool` (default `true`)
  - `memory.telemetry.usedThreshold: f32` (default 0.55)
  - `memory.telemetry.partialThreshold: f32` (default 0.35)
  - `memory.telemetry.minInjectionsForTrustAdjust: u32` (default 5)
  - `memory.telemetry.trustAdjustDelta: f32` (default 0.05)
  - `memory.telemetry.minResponseTokens: u32` (default 20) — skip below this
- Observer event `TelemetryObserver::on_use_classified(turn_id, counts, per_item)` fires once per turn after telemetry completes.
- Add CLI: `gaviero-cli memory utilization --scope repo --top 20` shows top-utilized / least-utilized memories for a scope.

---

## Expected output format from the implementation plan

For each of B5, B6 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from S, A, Phase 1 of Tier B, and earlier in Phase 2); sub-task order.

**3. Affected crates and modules.** Tree-style.

**4. New types, traits, public API.** For B5: `SessionConsolidate`, `Sleeptime` writer message variants, consolidator prompt constants, operation enum `ADD | MERGE | SUPERSEDE | DROP`. For B6: `retrieval_use` table schema, `memory_utilization` view, `TelemetryObserver` trait.

**5. Data-flow description.** Per-cadence flows. For B5: trigger → input-gather → LLM call → parse → apply operations → emit events. For B6: turn-complete → read-manifest → embed-response → classify → write-rows → aggregate-view-refresh.

**6. Schema / settings changes.** New tables (`retrieval_use`), settings additions.

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies. B5 specifically: session-consolidator, sleeptime-decay-sweep, sleeptime-near-dup-merge, sleeptime-promotion, sleeptime-trust-rescore as separate sub-tasks.

**8. Test strategy.** Unit + integration + **mandatory Tier 2 eval gate** for B5. Include: session-close-detection test, merge-does-not-destroy-user-memory test, sleeptime-idempotence test, dry-run test, B6-cosine-classification fixture test.

**9. Observability.** Tracing, metrics, observer events. B5 specifically needs an audit trail: every sleeptime operation logged with enough context to reverse manually if needed.

**10. Risks and rollback.** Per sub-task. For B5: what happens if consolidator LLM returns malformed JSON (fall through with warning; no data lost). For B6: what happens if embedder is unavailable at telemetry time (skip; log).

**11. Acceptance criteria.** Measurable.

**12. Open questions.** Decisions for the developer. Examples: session idle timeout value; sleeptime near-dup threshold (0.92 is a starting point); whether to run the consolidator on every session close or only sessions with >N turns; B6's cosine thresholds; whether to gate utilization-based trust adjustment on a minimum-turns-per-day threshold to avoid noise from quiet days.

---

## Evaluation requirements specific to this phase

- B5 requires Tier 2 non-regression after session-consolidator enabled. The consolidator prompt must be evaluated against 10+ held-out session transcripts for output quality (LLM-as-judge with manual cross-check).
- B5 sleeptime requires dry-run on a real populated `memory.db` with inspection of proposed operations before first enabled run. Acceptance: ≥95% of proposed merges manually judged correct; zero destructive operations on `user_remember` memories.
- B6 has no direct Tier 1/2 gate but contributes to the Tier 1 harness as a diagnostic (recall@K measured pre-telemetry and post-telemetry-driven trust updates should improve monotonically).

---

## Explicit non-goals for Phase 2

Do **not** plan the following here:

- Retrieval pipeline changes (covered in Phase 1 of Tier B)
- Embedding model swap (Phase 1 of Tier B)
- Cross-encoder reranker (Phase 1 of Tier B)
- Typed memory stores split (Records / History / Summaries) — Tier C1 (session summaries are first-class once C1 lands; until then they're a distinct type)
- `/forget` command with audit trail — Tier C2 (though B5 sleeptime's audit log lays the foundation)
- PageRank node-specificity — Tier C3
- Typed code graph edges — Tier C4
- KG node-doc schema — Tier D1 (B5's sleeptime step 5 is a stub until D1)
- Contextual Retrieval — Tier D2
- AGENTS.md / CLAUDE.md compat — Tier D3
- LLM-based response evaluator — explicitly rejected

---

## Acceptance criteria for Phase 2

Phase 2 is complete when all of the following are true:

1. Per-session consolidator runs on chat close and idle timeout, produces session summaries, emits MERGE/SUPERSEDE/DROP/ADD operations. Verified end-to-end against a seed session.
2. Sleeptime pass runs weekly (or on-demand), performs decay sweep + near-dup merge + promotion + trust re-scoring, and emits auditable observer events. Dry-run mode works. First-run confirmation works. Verified against a populated test `memory.db`.
3. Sleeptime never modifies `user_remember` memories without explicit user consent. Source-aware merging takes `user_remember` as ground truth when merging with an LLM-authored near-duplicate.
4. Retrieval-use telemetry classifies every injected memory per turn as Used / Partial / Unused within 100ms of turn completion. `retrieval_use` table grows correctly. `memory_utilization` view is accurate.
5. Sleeptime trust re-scoring consumes B6 utilization data and adjusts trust accordingly within the stated deltas and bounds.
6. TUI memory panel (from Tier A4) displays utilization indicator per row. Manifest-inspection view shows per-item classification.
7. `gaviero-cli memory utilization` and `gaviero-cli memory sleep --dry-run` CLI commands work.
8. No regression in Tier S, Tier A, or Tier B Phase 1 acceptance criteria.

---

## Anti-patterns to avoid

- **Letting the consolidator LLM write to `MemoryStore` directly.** It proposes; the writer task applies.
- **Making sleeptime operations irreversible without audit.** Every destructive op logs enough to reverse, feeding the Tier C2 audit trail.
- **Synchronous sleeptime blocking the UI.** Always in the writer task, always async.
- **Over-aggressive near-dup merge.** Conservative threshold (0.92); user-owned memories never merged without explicit consent.
- **Using an LLM for B6's usage judgment.** Cosine + substring is the point of B6; reaching for LLM-as-judge defeats the cost model.
- **Auto-deleting low-utilization memories.** Flag for review, don't delete.
- **Running telemetry on every turn regardless of response length.** Skip on short turns to avoid noise dominating the signal.
- **Assuming sleeptime runs daily.** It runs when idle; a user who closes Gaviero every night and reopens it without idle periods may go weeks without sleeptime. The weekly force-run safety net is load-bearing.

---

## Final instruction

Produce the implementation plan per **Expected output format** above, covering B5 and B6 in the recommended order (B5 → B6). B5 is the largest item in the entire Tier B — break it into shippable sub-tasks (session-consolidator first, then sleeptime-decay-sweep, then near-dup-merge, then promotion, then trust-rescore). Flag every decision the developer must make. If ambiguous, call it out as an open question.
