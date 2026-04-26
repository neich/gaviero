# Tier B Part 1 — Retrieval Pipeline Upgrades

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, data flow, migrations, evaluation strategy, task ordering, and acceptance criteria. Flag decisions that require human judgment. Do not produce full Rust code listings.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, and the relevant retrieval/embedding literature (RRF, cross-encoders, ModernBERT-family models). Do not explain these.

**This document covers Phase 1 of Tier B**, containing four items (B4, B1, B2, B3) that together upgrade Gaviero's retrieval pipeline from scoring through ranking. Phase 2 of Tier B (B5 session consolidator + sleeptime pass, B6 retrieval-use telemetry) is covered in a separate document (**Tier B Part 2 — Consolidation and Feedback Loop**). Treat Phase 1 as self-contained: the plan should be implementable without needing Phase 2 context.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (ratatui TUI editor) and `gaviero-cli` (headless runner). It orchestrates Claude Code (ACP), Codex, Ollama, and mock backends through a provider-agnostic `AgentBackend` trait. Agent writes flow through a `WriteGatePipeline`. Swarm orchestration runs a 6-phase pipeline.

Local-first, single-developer-oriented.

### Stack

- Rust 2024, tokio, petgraph, ropey, git2, ratatui 0.30
- SQLite (WAL mode) + sqlite-vec
- ONNX Runtime (`ort` crate) with nomic-embed-text-v1.5 (768 dim, current — B1 upgrades)
- tree-sitter 0.25, 16-language registry
- logos + chumsky + miette for the `.gaviero` DSL

### Crate layout

- `gaviero-core` (lib) — all runtime, no UI
- `gaviero-tui`, `gaviero-cli`, `gaviero-dsl`, `tree-sitter-gaviero`

### Memory subsystem state (assuming Tier S and Tier A are in place)

- 5-level scope: `Global → Workspace → Repo → Module → Run`
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- **Cascading retrieval with early exit at 0.70** (this is what B3 changes)
- RRF 0.7 vec / 0.3 fts (this is revisited in B2 but not directly changed)
- Composite scoring: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust_multiplier`
- 30-day half-life decay on recency (this is what B4 changes)
- SHA-256 + semantic near-dup dedup on writes
- **Writer task owns all writes** (Tier S2)
- **Chat auto-inject** at prompt assembly (Tier S1)
- **Retrieval manifest persistence** — every injection writes an `injection_manifests` row (Tier S4). 30-day retention. **Used by the eval harness for retrospective debugging and A/B analysis.**
- **Per-turn extractor** producing structured memories (Tier S3)
- **Turn annotations sidecar** providing high-signal LLM-flagged candidates (Tier A1)
- **`/remember` scope-corrected and visible** (Tier A2)
- **`source` and `trust` attributes** on all records (Tier A3)
- **TUI memory panel** for inspection and curation, with manifest inspection action (Tier A4)
- **Gaviero-as-MCP-server** exposing read-only tools (Tier A5)
- `Embedder` trait wraps `OnnxEmbedder` (current impl: nomic-embed-text-v1.5)

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle. This phase operates entirely within the retrieval path of the Memory layer — how candidates are ranked and how scoring reflects age and quality. Nothing here touches write path, consolidation, History, or Scratchpad.

### Agent session / prompt construction

`agent_session::build_turn` produces a `Turn` from `PlannerSelections`; chat goes through `acp::client::AcpPipeline::send_prompt` (with memory injection wired in from Tier S1).

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
11. **All memory writes go through the Tier S2 writer task.**
12. **MCP surface is read-only.**

### Decisions already taken (do not re-litigate)

- **Writer task, annotations sidecar, read-only MCP** — all architectural.
- **Three-cadence consolidation.** Per-turn in place (S3); session + sleeptime in Phase 2 of Tier B (B5).
- **No LLM writes.**
- **No graph-RAG / Neo4j / A-MEM memory evolution.**
- **LongMemEval and LOCOMO are not primary quality signals.** They're chat benchmarks; we measure on code-specific evals.

### Evaluation infrastructure (prerequisite or parallel to this phase)

A three-tier evaluation harness should exist before or during this phase:

- **Tier 1 — Retrieval smoke test.** 30–50 pinned `(query, expected_memory_id)` pairs from Gaviero's own use. Runs in <1 minute. Reports recall@1/5/10 and MRR. Runs on every PR.
- **Tier 2 — Code-specific memory eval.** 20–30 hand-curated multi-session coding scenarios. Each has a setup session that should produce memories and a test session that requires them. LLM-as-judge on response quality, with 20% manual cross-check.
- **Tier 3 — SWE-bench subset.** 20 issues from SWE-bench Verified on projects with history. A/B test memory on/off. Weekly.

**Every change in this phase must be gated by Tier 1 non-regression and (for B1, B2, B3) Tier 2 ≥ baseline − 1 scenario with at least one scenario improvement.** If the eval harness does not exist, building at least Tier 1 is the first sub-task of this phase.

The S4 retrieval manifests make the eval harness substantially cheaper to build: Tier 1's pinned (query, expected) pairs can be validated by replaying queries and inspecting the persisted manifest's candidate pool ranking rather than re-running retrieval from scratch.

---

## Phase 1 context and rationale

Phase 1 of Tier B delivers real but smaller retrieval-quality gains than Tier S or A. Every item requires careful evaluation — the gains are real but conditional on workload, and a poorly-chosen substitution can regress.

- **B4 (decay floor and type-based exemptions).** Prevents old decisions from silently decaying to unretrievable. Near-zero engineering cost. Ship first.
- **B1 (embedder upgrade to `gte-modernbert-base`).** Drop-in replacement keeping 768 dim. Expected +10–15 NDCG on code queries. Requires re-embedding. Highest single-swap value.
- **B2 (cross-encoder reranker).** Adds a rerank stage over top-50–100 → top-10. Expected +5–15 NDCG; removes sensitivity to RRF weight tuning. ~50–150ms added latency. Depends on B1 (evaluate the reranker against the upgraded embedder).
- **B3 (merged multi-scope retrieval).** Removes the 0.70 early-exit cascade; replaces with parallel multi-scope retrieval + RRF fusion + scope multiplier bias. Fixes a specific failure mode (Run scope spuriously winning over a better Repo memory).

**Dependencies and ordering within Phase 1:**

- **B1 is a prerequisite for B2** — you want to evaluate the reranker against the upgraded embedder, not the old one.
- **B3 is independent** of B1/B2 but becomes more effective once both are in place (the reranker compensates for more candidates in the merged pool).
- **B4 is independent** — can ship any time.

**Recommended sequence:** (Eval harness if absent) → B4 → B1 → B2 → B3.

---

## Items in this phase

### B4. Decay floor and type-based decay exemptions

**Problem.** The current exponential decay with 30-day half-life has no floor. A 180-day-old architectural decision has recency contribution of `0.15 * 0.015 ≈ 0.002` — effectively zero. A barely-relevant week-old lint observation outranks it. This silently makes old, high-value memories unretrievable.

**Architecture.**

- **Decay floor.** Modify the recency term in composite scoring:
  - Current: `recency = exp(-ln(2) / 30 * days_since_write)`
  - New: `recency = max(0.35, exp(-ln(2) / 30 * days_since_write))`
  - The floor value (0.35) is configurable as `memory.scoring.recencyFloor: f32`.
- **Type-based exemptions.** Certain memory types are reference facts, not event observations; they should not decay:
  - `decision` — architectural choices
  - `convention` — project-specific style patterns
  - `invariant` — properties that must hold
  - `preference` — explicit user preferences
- For these types: `recency = 1.0` (no decay applied). Settings: `memory.scoring.decayExemptTypes: [string]` (default `["decision", "convention", "invariant", "preference"]`).
- **Retirement path.** Exempt-from-decay means types like `decision` accumulate forever. To prevent contradictory old decisions from accumulating, lean on:
  - The per-session consolidator (Tier B Phase 2: B5) which can emit `SUPERSEDE` operations.
  - The sleeptime pass (Tier B Phase 2: B5) which does near-dup merge.
  - User-driven deletion via the TUI panel (Tier A4) or future `/forget` (Tier C2).
- **No retroactive change.** This affects scoring at retrieval time, not stored data. No migration needed.

**Benefits.**

- Prevents good old decisions from becoming unretrievable.
- Matches OMEGA and Letta defaults.
- Near-zero engineering cost.
- Aligns with the Generative Agents paper's observation that importance should protect against decay (Gaviero already applies importance weighting; this completes the idea).

**Risks.**

- **Reference memories accumulate forever.** Without retirement via SUPERSEDE (B5) or `/forget` (C2), old obsolete decisions outrank new correct ones. Track type=`decision` count per scope as a metric; if it climbs unboundedly, revisit.
- **Floor at 0.35 might be too generous.** If old junk memories start dominating retrieval, lower the floor. Make it easy to tune via settings.
- **Interacts with composite multipliers.** Existing scope/trust multipliers still apply; ensure the floor doesn't conflict.

**Alternatives considered and rejected.**

- *Importance-weighted decay rate* (higher-importance memories decay slower). More principled, more tuning. Floor achieves 80% of the benefit at 10% of complexity.
- *Git-commit-activity-weighted decay.* Clever but premature.
- *Keep linear decay to zero.* Causes the observed issue.

**Integration notes.**

- Update composite score computation in `memory/scoring.rs`. Add a unit test that verifies: a 180-day-old `decision`-type memory with high similarity outranks a 7-day-old `observation`-type memory with slightly higher similarity.
- When the panel (Tier A4) shows a memory, include a small decay indicator ("age: 120d • recency: 0.35 floor") for transparency.
- Decay floor + exemption is retrieval-time only; no rewrite of stored data.

---

### B1. Embedding model upgrade to `gte-modernbert-base`

**Problem.** Current embedder is nomic-embed-text-v1.5 (137M, 768 dim, Apache-2.0, ONNX). Solid general-text baseline but not trained on code; trails code specialists by 10–20 NDCG on NL2Code retrieval. Gaviero's memory content is heavy in code-adjacent prose (decisions about Rust patterns, lessons from debugging, module conventions), which this underperforms on.

**Architecture.**

- **Target model: `Alibaba-NLP/gte-modernbert-base`.** Apache-2.0, 149M params, 768 dim (same as nomic — **zero schema change**), 8192 context, official ONNX export available. ~2× slower CPU inference than nomic but still well under 100ms per embedding on modern hardware.
- Extend the existing `Embedder` trait (in `gaviero-core/src/memory/embedder.rs`) if needed to support instruction prefixes (gte-modernbert benefits from task-specific prefixes: `"search_query: "` and `"search_document: "`). Make prefixes configurable per embedder impl.
- Add `ModernBertEmbedder` struct alongside `OnnxEmbedder` (or generalize `OnnxEmbedder` if the ONNX loading/tokenizer logic can be reused). Use `ort` crate as today; tokenizer via `tokenizers` crate with the model's vocab.
- Model file discovery via the existing `model_manager.rs`: download from HuggingFace or serve from user-local cache at `~/.cache/gaviero/models/gte-modernbert-base/`. Reuse whatever flow nomic uses.
- **Re-embedding migration.** Existing `memory.db` has embeddings generated by nomic. A one-time pass re-embeds all records with the new model. Migration steps:
  1. Check schema: add `_gaviero_meta.embedder_model = "nomic-embed-text-v1.5"` retrospectively if absent.
  2. On first startup after upgrade, detect mismatch between `_gaviero_meta.embedder_model` and configured embedder.
  3. Offer migration: "Re-embed N records with gte-modernbert-base? Takes ~X minutes." User opt-in.
  4. On confirmation: parallelized re-embed (tokio bounded semaphore), update vectors in `vec_memories_scoped` table, update `_gaviero_meta.embedder_model`.
  5. Migration is reversible by re-running against nomic; backup of `memory.db` is taken before migration starts.

**Benefits.**

- Projected +10–15 NDCG@10 on code-adjacent queries based on CoIR-style benchmarks.
- Same dimension → no schema change, no vector column-width migration.
- Modern architecture (ModernBERT) carries improvements in tokenization efficiency and long-context handling.
- Zero runtime-cost increase beyond the 2× inference time, which matters only during bulk re-embed.

**Risks.**

- **Benchmark numbers don't transfer cleanly.** CoIR-family leaderboards are noisy and sometimes overfit. Validate against Tier 1 retrieval smoke test on Gaviero's own data *before* committing. If Tier 1 regresses, abort and investigate.
- **Re-embedding is atomic but not reversible without backup.** Take a backup; document rollback.
- **Prefix handling.** gte-modernbert may benefit from `"search_query: "` and `"search_document: "` prefixes. Query embeddings at injection time get the query prefix; stored embeddings get the document prefix. If prefix logic regresses existing retrieval (asymmetric query/doc prefix), may need to re-embed with different prefix choice.
- **Tokenizer performance.** ModernBERT tokenizer may be slower than nomic's. Profile cold-start and warm embedding speeds before shipping.

**Alternatives considered and rejected.**

- *`Qwen/Qwen3-Embedding-0.6B`* — higher ceiling, but 1024-dim native (schema change), 4× slower CPU, larger model. Defer 6–12 months.
- *Stay with nomic-embed-text-v1.5.* Safe but leaves quality on the table for zero ongoing cost.
- *Separate code embedder + prose embedder with routing.* More infrastructure, marginal gain vs single modern general model. Gaviero's memory content isn't pure code anyway.
- *API-based embedders (Voyage code-3, Cohere embed-4).* Violates local-first.

**Integration notes.**

- After re-embedding, run Tier 1 smoke test. Compare pre/post. If recall@5 drops more than 2 points on any query class, hold and investigate.
- Update `ui.memoryPanel` to show embedder version somewhere (small, for debugging).
- Do not remove nomic support from the codebase; keep as fallback/comparison embedder behind a config flag.
- Settings: `memory.embedder.model: "nomic" | "gte-modernbert"` (default `"gte-modernbert"` after migration).
- Consider extending the `Embedder` trait now to take an `EmbeddingPurpose` enum (`Query | Document`) so the trait naturally supports prefix-requiring models.
- S4 manifests produced before B1 migration will have stale `embedder_name` fields. Do not retroactively rewrite. The eval harness must account for this when replaying manifests across embedder versions.

---

### B2. Cross-encoder reranker

**Problem.** After hybrid retrieval (RRF fusion of vec + FTS5), the top candidates are ranked by a linear composite score. For marginal cases (similarity 0.6–0.8 range), the ranking is noisy — a truly-relevant memory at rank 7 might be exactly as valuable as a less-relevant one at rank 3. A cross-encoder reranker that jointly scores query + candidate is the standard way to sharpen this regime.

**Architecture.**

- **Target model: `Alibaba-NLP/gte-reranker-modernbert-base`.** Apache-2.0, ~150M params, official ONNX. Same family as the B1 embedder; shares tokenizer infrastructure. ~50–150ms CPU for 100 candidates.
- Insert rerank between the hybrid retrieval stage and the final top-K return:
  1. Hybrid retrieval (vec + FTS5 + RRF + composite) returns top-N candidates where N is ~50–100 (configurable).
  2. Reranker scores each `(query, candidate.text)` pair jointly.
  3. **Blend the reranker score with the existing composite score** rather than replacing. A defensible blend: `final_score = 0.6 * rerank_score + 0.4 * composite_score`. This preserves the scope-multiplier and trust-multiplier semantics while letting the reranker dominate where it has signal.
  4. Sort by `final_score`, return top-K.
- Rerank runs **outside** the SQLite lock. The candidate set is already materialized in memory at this point; reranker just needs text.
- Add a `Reranker` trait to `gaviero-core/src/memory/reranker.rs`:
  ```
  trait Reranker: Send + Sync {
      async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>>;
  }
  ```
- `ModernBertReranker` impl using `ort` + `tokenizers`. Reuses the model manager pattern from embedder.
- **Feature gating.** `memory.reranker.enabled: bool` (default `false` initially; flip to `true` after evaluation shows gain).
- Settings:
  - `memory.reranker.model: "gte-reranker-modernbert" | "none"` (default `"gte-reranker-modernbert"`)
  - `memory.reranker.candidatePoolSize: usize` (default 50)
  - `memory.reranker.blendWeight: f32` (default 0.6, where 1.0 = pure reranker, 0.0 = pure composite)

**Benefits.**

- Projected +5–15 NDCG improvement, more on messy/underspecified queries.
- Effectively removes sensitivity to RRF weight tuning — the reranker compensates for suboptimal fusion weights.
- Matches standard practice in mature retrieval systems.

**Risks.**

- **50–150ms per retrieval.** Imperceptible on chat, but visible in MCP `memory_search` tool calls inside long agent loops. Profile against target latency budget.
- **Memory footprint.** Second ONNX model in process (~150MB for gte-reranker-modernbert). Verify resident set stays reasonable.
- **Miscalibration.** A reranker trained on web prose may underperform RRF alone on very code-identifier-heavy queries. Gate with evaluation; be willing to disable.
- **Blend weight tuning.** 0.6/0.4 is a starting point; may need per-query-class tuning. Defer to measurement.

**Alternatives considered and rejected.**

- *ColBERT-style late interaction.* More complex indexing, larger storage; overkill at Gaviero's scale.
- *LLM-as-reranker.* Too slow and expensive for per-query rerank.
- *Stick with RRF and tune weights per query class.* Possible but less robust than a learned reranker.
- *`BAAI/bge-reranker-v2-m3`.* 568M, more battle-tested but slower and larger. Keep as a fallback option exposed via settings.

**Integration notes.**

- The reranker runs on all retrieval paths: chat injection (Tier S1), MCP `memory_search` tool (Tier A5), memory panel search (Tier A4). Centralize so a change to rerank affects all.
- S4 manifests must record both the composite score AND the rerank score per candidate when the reranker is enabled, so the panel's inspect-manifest view can show both. Extend the manifest schema to include optional `rerank_score: f32` per candidate.
- When reranker is disabled (settings off or model file missing), fall back silently to composite-only ranking — never error retrieval.
- Consider a warmup step at workspace-open time (one dummy rerank call) to pay the model-load cost up front rather than on the first real query.

---

### B3. Remove 0.70 early-exit; merged multi-scope retrieval

**Problem.** The current cascading retrieval tries scopes narrowest-first (`Run → Module → Repo → Workspace → Global`) and early-exits when the best score exceeds 0.70. This has two failure modes:

1. **Run scope spuriously wins.** A fresh Run-scope memory with 0.72 similarity wins over a Repo-scope memory with 0.90 similarity, because the broader scope is never consulted.
2. **Hard threshold is fragile.** 0.70 is domain-dependent; changing embedders (B1) shifts the whole similarity distribution.

**Architecture.** Replace the cascade with parallel multi-scope retrieval + scope-aware fusion:

1. Given a query, retrieve top-K candidates from each scope level in parallel (tokio joins).
2. Each scope's candidates carry their scope identity.
3. Merge all candidates into a single pool.
4. Apply the existing composite score with `scope_multiplier` — this is where scope bias lives now, as a soft signal rather than a hard gate.
5. Pass the merged pool through the reranker (B2, if enabled).
6. Return top-K overall.

Scope multipliers (existing formula's term) become load-bearing. Suggested defaults:
- Run: 1.10 (slight recency bias)
- Module: 1.05
- Repo: 1.00 (baseline)
- Workspace: 0.95
- Global: 0.85

These are starting points; evaluate with Tier 1 and adjust.

**Benefits.**

- Eliminates the "Run drowns Repo" failure mode from the diagnosis.
- Aligns with current best practice (HippoRAG 2 explicitly warns against hard graph/scope gating).
- More debuggable — the TUI panel (Tier A4) now shows a single merged ranking with scope badges rather than "this scope had nothing, try next."
- Embedder changes (B1) don't break a fixed 0.70 threshold.

**Risks.**

- **More candidates per query = higher baseline latency.** Retrieving top-20 from 5 scopes = 100 candidates, then reranker runs on them. With reranker: 100 × 10ms = 1s. Mitigate via: (a) per-scope top-K limit (e.g., top-10 per scope, 50 total), (b) reranker pool size cap (B2 already has this).
- **Scope multiplier miscalibration.** Setting multipliers wrong can either over-weight Run (same issue as before) or over-weight Global (cross-project leakage). Evaluate carefully.
- **Global scope retrieval cost.** The global memory.db is a separate file; retrieving from it adds an I/O path. Ensure connections are pooled/cached.

**Alternatives considered and rejected.**

- *Keep cascade but lower threshold.* Doesn't fix the structural problem.
- *Cascade only for specific query types.* Adds branching, unclear gain.
- *Cascade with overlap* (always include next scope even if current succeeds). Workable but strictly worse than flat merge + reranker.

**Integration notes.**

- Per-scope top-K retrieval can happen concurrently (`tokio::join!` or `FuturesUnordered`).
- Gaviero currently has separate `memory.db` per workspace + a global one. The retrieval path must handle both. Ensure connection handles are held at the `Workspace` level, not per-query.
- Observer event `AcpObserver::on_memory_injected` (added in Tier S1) should now carry scope-distribution info for the panel.
- S4 manifests' `candidate_pool` entries already carry per-item `scope` — this becomes especially useful after B3 because the panel can visualize cross-scope balance per turn.

---

## Expected output format from the implementation plan

For each of B4, B1, B2, B3 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from Tier S, Tier A, or earlier items in this phase); recommended sub-task order.

**3. Affected crates and modules.** Tree-style.

**4. New types, traits, public API.** High-level signatures; for B1/B2 specifically the `Embedder` and `Reranker` trait signatures and their implementations.

**5. Data-flow description.** Which task, which lock, which channel. For B3 specifically: parallel-scope retrieval → merged pool → rerank → composite.

**6. Schema / settings changes.** Settings.json additions, model file layouts, any manifest-schema extensions needed (for B2 adding `rerank_score` to candidate pool entries).

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies. B1 specifically should call out the re-embedding migration as its own sub-task with progress UI and backup handling.

**8. Test strategy.** Unit tests + integration tests + **mandatory Tier 1 and Tier 2 eval gates** per change. Specify what the gate thresholds are. For B2: rerank ablation (rerank off vs on) with NDCG delta requirement. For B1: side-by-side embedder comparison report.

**9. Observability.** Tracing, metrics, observer events. Per-retrieval latency broken into: hybrid-search, rerank, scoring. Per-scope retrieval counts. Embedder-version metric.

**10. Risks and rollback.** Per sub-task: failure modes and reversal.

**11. Acceptance criteria.** Measurable conditions.

**12. Open questions.** Decisions for the developer. Examples: whether to keep nomic support indefinitely or deprecate after a grace period; reranker blend weight starting value; scope multipliers for B3; decay floor value; whether B3's merged pool should apply the trust multiplier after or before the rerank blend.

---

## Evaluation requirements specific to this phase

**Every Phase 1 change must be gated by evaluation.** The plan must specify:

- Tier 1 smoke test run before and after each change; regression = more than 2 points drop in recall@5 on any query class.
- Tier 2 code-specific eval run before and after B1, B2, B3; regression = more than 1 scenario regressing with zero improvements.
- B1 requires side-by-side comparison of nomic vs gte-modernbert on the pinned query set; user must review the comparison report before committing migration.
- B2 requires a rerank ablation: retrieval with rerank off vs on; user reviews NDCG delta on the pinned set.
- B3 requires a specific regression test for the "Run drowns Repo" failure mode: a fixture where the correct answer is in Repo scope and a marginal Run-scope match would have early-exited.

If the eval harness doesn't exist yet, building Tier 1 is the first sub-task of this phase. Tier 2 can be built lazily as scenarios accumulate.

---

## Explicit non-goals for Phase 1

Do **not** plan the following here:

- Per-session consolidator and sleeptime pass — Phase 2 (B5)
- Retrieval-use telemetry — Phase 2 (B6)
- Typed memory stores split (Records / History / Summaries) — Tier C1
- `/forget` command — Tier C2
- HippoRAG-style node-specificity weighting in PageRank — Tier C3
- Typed code graph edges — Tier C4
- Pluggable embedder/reranker traits beyond what B1/B2 require for these specific models — Tier C5
- KG node-doc schema — Tier D1
- Contextual Retrieval for docs — Tier D2
- AGENTS.md / CLAUDE.md compat — Tier D3
- New embedding models beyond `gte-modernbert-base` — future.
- LLM-based rerankers — explicitly rejected.
- LongMemEval or LOCOMO as primary quality signals.

---

## Acceptance criteria for Phase 1

Phase 1 is complete when all of the following are true:

1. Memories of type `decision`, `convention`, `invariant`, `preference` do not decay; floor of 0.35 applies to all others. A specific integration test verifies that a 180-day-old high-similarity decision outranks a 7-day-old slightly-higher-similarity observation.
2. `gte-modernbert-base` is the default embedder; migration from nomic-embed-text-v1.5 is documented, tested, and reversible. Tier 1 retrieval smoke test shows non-regression or improvement.
3. Cross-encoder reranker is enabled by default after evaluation confirms gain; Tier 1 recall@5 improves by at least 3 points; retrieval latency budget stays under 250ms at P95. Falls back silently to composite-only ranking when disabled.
4. Retrieval is merged-multi-scope (no 0.70 early-exit); scope bias is a soft multiplier only; "Run drowns Repo" regression test passes.
5. S4 manifests include rerank scores per candidate (when reranker enabled) and scope distribution for the merged pool (after B3).
6. TUI memory panel (from Tier A4) continues to function correctly, now showing reranker scores, merged multi-scope candidates, and decay-floor state.
7. No regression in Tier S or Tier A acceptance criteria.

---

## Anti-patterns to avoid

- **Shipping any Phase 1 change without eval gate.** Every change is evaluated against Tier 1; B1/B2/B3 additionally against Tier 2.
- **Replacing existing composite score entirely with rerank score.** Blend, don't replace.
- **Hardcoding the embedder or reranker model.** Settings-configurable; trait-abstracted (the abstraction itself stays minimal in this phase; full pluggable-traits formalization is Tier C5).
- **Forgetting to re-run Tier 1 after B1 migration.** The migration is not complete until the eval has been run post-migration.
- **Baking the 0.70 threshold somewhere else after removing it from the cascade.** No magic numbers for scope gating; the multiplier is the only knob.
- **Letting reranker latency balloon at the MCP call path.** MCP `memory_search` tools have an implicit latency budget (subprocess agents expect <200ms); monitor and cap.

---

## Final instruction

Produce the implementation plan per **Expected output format** above, covering B4, B1, B2, B3 in the recommended order (eval harness if needed → B4 → B1 → B2 → B3). Include evaluation gates as first-class tasks — if Tier 1 doesn't exist, building it is sub-task 0. Flag every decision the developer must make. If ambiguous, call it out as an open question rather than deciding.
