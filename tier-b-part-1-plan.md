# Tier B Part 1 — Retrieval Pipeline Upgrades: Implementation Plan

Recommended sequence: **Eval harness (T0) → B4 → B1 → B2 → B3**.

---

## T0. Eval harness (Tier 1 smoke test) — prerequisite

**Status check.** No eval harness currently in the tree. Build Tier 1 first; build Tier 2 lazily as scenarios accumulate.

**Architecture.** A new binary subcommand `gaviero-cli memory eval` (clap subcommand under existing CLI) plus a fixture file `crates/gaviero-core/eval/tier1.jsonl`. Each line: `{ "id": str, "query": str, "expected_memory_id": i64, "scope": str, "tags": [str] }`. The harness:

1. Opens a designated eval workspace (`--eval-db <path>` flag) so the dev's real workspace DB is never touched.
2. For each fixture, calls the existing search path (B3 will swap in merged-multi-scope) and records the rank of `expected_memory_id` in the candidate pool.
3. **Reuses S4 manifests for cheap re-evaluation.** When `--from-manifests` flag is passed, the harness reads `injection_manifests` rows by tag and rescores their stored candidate pools under the current scoring formula — no embeddings re-run, no LLM in the loop.
4. Reports recall@1, recall@5, recall@10, MRR, and per-`tags` breakdown. Exits non-zero if any class drops more than 2 points vs the prior run (stored in `eval/tier1-baseline.json`).

**Affected files.**
- `crates/gaviero-cli/src/main.rs` — new `EvalArgs` subcommand wiring.
- `crates/gaviero-core/src/memory/eval.rs` — new module owning fixture loader, runner, recall/MRR computation, baseline diff.
- `crates/gaviero-core/eval/tier1.jsonl` — fixture, seeded with ~30 pairs from the developer's own session manifests.
- `crates/gaviero-core/eval/tier1-baseline.json` — generated; checked in.

**Task breakdown.**
- T0a (S) — Fixture loader + recall/MRR computation, unit tested on synthetic data.
- T0b (S) — `gaviero-cli memory eval` subcommand wiring, both `--live` and `--from-manifests` modes.
- T0c (M) — Seed fixture: replay 30 representative manifests from a populated dev workspace, hand-pick `expected_memory_id` per query.
- T0d (S) — Baseline file generation; CI gate that fails on >2 point regression.

**Open question.**
- Do we want a CI run (GitHub Actions) on every PR, or a make-target the dev runs manually? CI requires the eval DB to be checked in or built from a script — checking in a small `.db` is simplest.

---

## B4. Decay floor and type-based decay exemptions

### 1. Summary
Replace the unbounded exponential decay with a configurable floor, and exempt reference-fact memory types (`decision`, `convention`, `invariant`, `preference`) from decay entirely. Retrieval-time only — no stored data changes, no migration. User-visible effect: an old high-importance decision keeps surfacing instead of silently fading. Non-goals: no retirement/SUPERSEDE logic (B5), no importance-weighted decay rate.

### 2. Dependencies and ordering
- T0 must be in place to gate the change.
- No dependencies on B1/B2/B3.
- **Blocker discovered:** the existing `MemoryType` enum at [scope.rs:381](crates/gaviero-core/src/memory/scope.rs#L381) only has `Factual | Procedural | Decision | Pattern | Gotcha`. The plan requires `convention`, `invariant`, `preference`. These are listed in the Tier S extractor rubric but never landed as enum variants. **Sub-task B4a expands the enum.**

### 3. Affected crates and modules
```
crates/gaviero-core/src/memory/
├── scope.rs              [modify] expand MemoryType enum
├── scoring.rs            [modify] recency_factor() helper + exemption check
├── store.rs              [modify] load_scoped_memory passes memory_type to scorer
└── trust_defaults.rs     [touch]  if any default-type wiring needs updating
crates/gaviero-core/src/workspace.rs   [modify] new settings keys + defaults
crates/gaviero-tui/src/panels/memory_panel.rs   [modify] decay indicator on rows
```

### 4. New types, traits, public API
- `MemoryType` enum gains `Convention, Invariant, Preference, Lesson, Error` (the latter two from S3 rubric — fold in now to avoid a second migration). `as_str` / `parse_str` updated. Pre-existing rows that wrote unknown strings already fall through `parse_str`'s `_ => Factual` arm — unchanged.
- New helper in `scoring.rs`:
  ```
  pub fn recency_factor(days_since_access: f64, memory_type: MemoryType, floor: f32) -> f32
  ```
  Returns `1.0` for exempt types; `(exp(-ln2/30 * days)).max(floor)` otherwise.
- `score_with_trust_score` signature gains `memory_type: MemoryType`. The legacy `score()` function is updated to delegate to the new path with `MemoryType::Factual` so test callers continue to compile.

### 5. Data-flow description
Pure read-side change. `MemoryStore::load_scoped_memory` already loads `memory_type` from the row; threading it into the scorer is one extra argument. No locks, no new tasks, no channels.

### 6. Schema / settings changes
- **No SQL schema change.** Existing `memory_type` TEXT column already stores arbitrary strings.
- Settings additions (defaults in [workspace.rs](crates/gaviero-core/src/workspace.rs)):
  - `memory.scoring.recencyFloor: f32` — default `0.35`.
  - `memory.scoring.decayExemptTypes: [string]` — default `["decision","convention","invariant","preference"]`.

### 7. Task breakdown
| ID | Description | Size | Deps |
|----|-------------|------|------|
| B4a | Expand `MemoryType` enum + `as_str`/`parse_str`; update `MEMORY_TYPES` test list | S | — |
| B4b | Add `recency_factor()` + plumb `memory_type` into `score_with_trust_score`; thread through `load_scoped_memory` | S | B4a |
| B4c | Settings keys + defaults; resolve at `MemoryStore::open` time, store on store struct | S | B4b |
| B4d | Panel row decoration: append `[age 120d • floor]` when floor applies | S | B4c, A4 |
| B4e | **Tier 1 eval gate** — non-regression check; specific scenario test that 180-day decision outranks 7-day observation | S | T0, B4b |

### 8. Test strategy
- Unit: `recency_factor` returns `1.0` for exempt types, `floor` for very old non-exempt, `>floor` for fresh.
- Integration in [store.rs](crates/gaviero-core/src/memory/store.rs) tests: insert two memories with synthetic timestamps (`updated_at` set via direct UPDATE), assert ranking matches the plan's expectation.
- Tier 1 gate: `cargo run -p gaviero-cli -- memory eval --from-manifests` must show no recall@5 drop > 2 points.

### 9. Observability
- `tracing::trace!(target: "memory_scoring", memory_id, recency, memory_type, "scoring")` per candidate.
- New metric counter `memory_scoring_decay_floor_applied_total` — increments when the floor binds, so we can see whether the floor is load-bearing or vestigial.
- Histogram `memory_scoring_recency_distribution` (bucketed).

### 10. Risks and rollback
- **Risk: reference memories accumulate forever.** Track `decision`-count per scope; revisit if it grows >10×. Rollback: set `decayExemptTypes = []` in settings — instant revert without deploy.
- **Risk: floor too generous.** Lower floor in settings; no code change.

### 11. Acceptance criteria
- 180-day-old `decision` (sim 0.85) outranks 7-day-old `factual` (sim 0.86) in a deterministic test.
- Tier 1 recall@5 unchanged or improved.
- Settings override flips behavior without recompilation.

### 12. Open questions
- **Decay floor value.** 0.35 is the plan's suggestion; should we start at 0.25 to be conservative until we have Tier 2 signal? **Developer decides.**
- **Should `lesson`, `error` be exempt or just floor-bound?** Plan lists only the four canonical types. Recommend: **floor only** for `lesson`/`error` — they describe past events, not invariants.
- **Should `gotcha` (pre-existing) be exempt?** It's reference-like ("X breaks if Y"). Recommend: **yes**, add to default exempt list.

---

## B1. Embedder upgrade to `gte-modernbert-base`

### 1. Summary
Swap the default embedder from nomic-embed-text-v1.5 to `Alibaba-NLP/gte-modernbert-base`. Same dim (768) → no schema change. Run a one-time, opt-in re-embed migration with backup. User-visible effect: better recall on code-adjacent queries; one-time "Re-embed N records?" prompt on first launch post-upgrade. Non-goals: tokenizer rewrite, prefix-strategy research, multi-embedder routing.

### 2. Dependencies and ordering
- T0 mandatory (so we can compare nomic vs gte side-by-side).
- B4 should land first so the eval gate runs against the new scoring.
- No dependency on B2/B3.
- Blocker for B2 (reranker is evaluated against the upgraded embedder).

### 3. Affected crates and modules
```
crates/gaviero-core/src/memory/
├── embedder.rs           [modify] add EmbeddingPurpose enum if not implicit
├── modernbert_embedder.rs[new]    GteModernBertEmbedder struct
├── onnx_embedder.rs      [modify] keep nomic impl behind config
├── model_manager.rs      [modify] add GTE_MODERNBERT_BASE constant
├── mod.rs                [modify] embedder factory: settings → impl
├── schema.rs             [modify] _gaviero_meta.embedder_model row
└── reembed_migration.rs  [new]    one-shot re-embed routine
crates/gaviero-tui/src/app/controller.rs   [modify] migration prompt UI
crates/gaviero-cli/src/main.rs             [modify] --reembed flag
```

### 4. New types, traits, public API
- Existing `Embedder` trait at [embedder.rs](crates/gaviero-core/src/memory/embedder.rs) already has `embed_query` / `embed_document` overrides. **Sufficient as-is** — no `EmbeddingPurpose` enum needed; the two methods carry the purpose.
- `pub struct GteModernBertEmbedder` mirroring `OnnxEmbedder` shape: ort `Session`, `Tokenizer`, `model_id() -> "gte-modernbert-base"`, prefixes `"search_query: "` / `"search_document: "`.
- `pub fn build_embedder(model_setting: &str, manager: &ModelManager) -> Result<Arc<dyn Embedder>>` — single factory used by both `MemoryStore::open` and the writer task.
- New module `reembed_migration::reembed_all(store: &MemoryStore, new_embedder: &dyn Embedder, progress: impl Fn(usize, usize)) -> Result<ReembedReport>`.

### 5. Data-flow description
- **Cold start.** `Workspace::open` resolves `memory.embedder.model`; `build_embedder` returns the configured impl. `MemoryStore::open` reads `_gaviero_meta.embedder_model`; if mismatch, sets `pending_reembed = true` flag on the store.
- **Migration trigger.** TUI controller, on `Event::MemoryReady`, checks `store.needs_reembed()`. If true, posts a system message: "Re-embed N records with gte-modernbert-base? Type `/reembed` to start." (Manual trigger keeps it intentional.)
- **Migration run.** `/reembed` command kicks `reembed_all` on a tokio task with bounded `Semaphore::new(num_cpus)`. Each batch: embed outside lock, then `UPDATE memories SET embedding = ?, model_id = ? WHERE id = ?` and `vec_memories_scoped` re-insert in one short critical section. Progress streams via observer event `Event::ReembedProgress { done, total }` to the panel.
- **Backup.** Before the first batch, `std::fs::copy(memory.db, memory.db.bak-<timestamp>)`. Documented rollback: stop Gaviero, `mv memory.db.bak-* memory.db`, set `memory.embedder.model = "nomic"`.

### 6. Schema / settings changes
- `_gaviero_meta` row: `('embedder_model', 'gte-modernbert-base')` — already exists per Tier A migration metadata pattern; reuse.
- Settings:
  - `memory.embedder.model: "nomic" | "gte-modernbert"` — default `"gte-modernbert"` **after migration ships** (default `"nomic"` initially in the PR that lands the code so existing users opt in).
  - `memory.embedder.reembedBatchSize: usize` — default 32.

### 7. Task breakdown
| ID | Description | Size | Deps |
|----|-------------|------|------|
| B1a | Add `GTE_MODERNBERT_BASE` `ModelInfo`; verify download via `model_manager` | S | — |
| B1b | `GteModernBertEmbedder` impl; tokenizer wiring; CPU inference test under 100ms | M | B1a |
| B1c | `build_embedder` factory; thread setting through `MemoryStore::open`, writer task, retrieval path | S | B1b |
| B1d | `reembed_migration::reembed_all` with bounded concurrency + backup | M | B1c |
| B1e | TUI `/reembed` command + progress event + panel indicator | S | B1d, A4 |
| B1f | CLI `gaviero-cli --reembed` flag | S | B1d |
| B1g | **Side-by-side eval report** — `gaviero-cli memory eval --compare nomic gte-modernbert`; user reviews before merge | M | T0, B1d |

### 8. Test strategy
- Unit: tokenizer round-trip; dimensions(); model_id().
- Integration: re-embed on a 100-row fixture DB completes; `vec_memories_scoped` row count unchanged; backup file present.
- **Tier 1 eval gate**: post-migration smoke test must show non-regression. `--compare` mode produces a report file; merge gated on developer approval (no automated greenlight).
- Tier 2 (if scenarios exist): each scenario re-run on both embedders; LLM-as-judge averages logged.

### 9. Observability
- `embedder_inference_duration_seconds{model="..."}` histogram.
- `memory_reembed_progress{done,total}` gauge during migration.
- `memory_embedder_version` gauge (label only) — visible at panel debug line.

### 10. Risks and rollback
- **Tokenizer mismatch.** ModernBERT uses a different tokenizer; broken downloads silently produce zeros. Verify by computing a known query embedding and asserting non-zero norm.
- **Migration interrupted.** `reembed_all` writes per-row idempotently (`UPDATE ... WHERE id = ?` is safe to retry); `_gaviero_meta.embedder_model` flips only after the final batch. Resuming = re-running.
- **Rollback.** Restore `.bak`, flip setting, restart. Documented in `MIGRATION.md`.

### 11. Acceptance criteria
- New install: launches with gte-modernbert by default.
- Existing install with nomic data: prompted to migrate; migration completes; `_gaviero_meta` updated; old vectors replaced.
- Tier 1 recall@5 ≥ baseline; ideally ≥ +5 points.
- Backup file present after migration; restore procedure tested.

### 12. Open questions
- **Default flip timing.** Land code with default = `"nomic"`, flip default to `"gte-modernbert"` in a follow-up PR after eval is green? **Recommend: yes**, two PRs.
- **Prefix strategy.** Use `"search_query: "` / `"search_document: "` (matches gte's training)? Or no prefix? Need a small ablation in B1g. **Developer decides** based on the comparison report.
- **Keep nomic indefinitely?** Plan says "as fallback/comparison". Recommend keeping the code path but not the model file — model only downloaded if `model = "nomic"` is explicitly set.

---

## B2. Cross-encoder reranker

### 1. Summary
Add a `Reranker` trait + `ModernBertReranker` impl that scores `(query, candidate_text)` jointly across the top-N (default 50) candidates from hybrid retrieval, then blends with the existing composite score (default 0.6 rerank / 0.4 composite). Disabled by default until eval shows gain. User-visible effect: marginal-similarity queries return a clearly better top-3. Non-goals: ColBERT, LLM rerank, per-query-class blend tuning.

### 2. Dependencies and ordering
- B1 must be merged so the reranker is evaluated against the upgraded embedder.
- T0 mandatory; the rerank ablation is a Tier 1 gate.
- Independent of B3, but interaction matters: B3 produces a larger merged pool (pre-rerank), so the reranker pool size cap (`candidatePoolSize`) is the load-bearing knob.

### 3. Affected crates and modules
```
crates/gaviero-core/src/memory/
├── reranker.rs           [new]    trait Reranker + ModernBertReranker
├── model_manager.rs      [modify] GTE_RERANKER_MODERNBERT_BASE constant
├── retrieval.rs          [modify] insert rerank stage in retrieve_for_chat
├── store.rs              [modify] cascading_search returns un-truncated pool to caller; rerank applies above
├── scoring.rs            [modify] blend_rerank(rerank_score, composite_score, w) helper
├── schema.rs             [modify] injection_manifests.candidate_pool entries gain rerank_score
└── mod.rs                [modify] re-export Reranker, ModernBertReranker
crates/gaviero-core/src/mcp/tools.rs           [modify] memory_search applies same rerank
crates/gaviero-tui/src/panels/memory_panel.rs  [modify] inspect-manifest shows rerank column
```

### 4. New types, traits, public API
```rust
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>>;
    fn model_id(&self) -> &str;
}

pub struct ModernBertReranker { /* ort Session, Tokenizer */ }

pub struct RerankConfig {
    pub enabled: bool,
    pub pool_size: usize,
    pub blend_weight: f32,        // 0..=1, weight on rerank score
    pub max_latency_ms: u64,      // soft budget; on exceed, log and continue
}

pub fn blend_rerank(rerank: f32, composite: f32, w: f32) -> f32 {
    w * rerank + (1.0 - w) * composite
}
```
- `MemoryStore::cascading_search` already returns truncated top-K; add `cascading_search_with_pool` that returns the merged pool *before* truncation. Rerank stage operates above the store API, in `retrieval::retrieve_for_chat`.

### 5. Data-flow description
```
chat injection / mcp memory_search / panel search
  └─ retrieval::retrieve_for_chat
       ├─ store.cascading_search_with_pool(query, scopes, K=pool_size)   [SQLite lock; embed outside]
       ├─ if reranker.enabled:
       │     reranker.rerank(query, &texts)                              [ONNX, no SQLite lock]
       │     blend final = w*rerank + (1-w)*composite
       └─ sort by final, take top-K_user
```
Reranker runs entirely outside the SQLite mutex. Models are warmed at workspace-open (one dummy 1-candidate call) to amortize the ~200ms first-load cost.

### 6. Schema / settings changes
- Settings:
  - `memory.reranker.enabled: bool` — default `false` (flip to `true` in a follow-up PR after eval).
  - `memory.reranker.model: "gte-reranker-modernbert" | "none"` — default `"gte-reranker-modernbert"`.
  - `memory.reranker.candidatePoolSize: usize` — default `50`.
  - `memory.reranker.blendWeight: f32` — default `0.6`.
  - `memory.reranker.maxLatencyMs: u64` — default `200`.
- Manifest schema (S4): each `candidate_pool` entry gains optional `rerank_score: Option<f32>`. Versioning already exists via `scoring_formula_version`; bump to `2` when reranker enabled. Old manifests (`v1`) replay correctly because the field is optional.

### 7. Task breakdown
| ID | Description | Size | Deps |
|----|-------------|------|------|
| B2a | `Reranker` trait + `ModernBertReranker` impl + model download | M | B1a |
| B2b | `cascading_search_with_pool` returning untruncated merged pool | S | — |
| B2c | Rerank stage in `retrieval::retrieve_for_chat`; warmup on workspace open | M | B2a, B2b |
| B2d | Manifest extension: `rerank_score` per candidate; CLI `manifest --turn` shows it | S | B2c, S4 |
| B2e | Wire MCP `memory_search` and panel search through the same retrieval entry point so rerank applies uniformly | S | B2c |
| B2f | **Rerank ablation report** — `gaviero-cli memory eval --rerank=on,off`; user reviews NDCG delta | M | T0, B2c |
| B2g | Flip default to `enabled=true` in follow-up PR after B2f green | S | B2f |

### 8. Test strategy
- Unit: blend_rerank algebra; pool size cap; latency-budget bypass keeps composite-only ranking.
- Integration: a fixture where the rerank-correct top-1 is at composite rank 5; rerank promotes it to top-1.
- **Tier 1 ablation gate**: recall@5 must improve by ≥3 points with reranker on; latency P95 ≤250ms.
- Snapshot tests on the panel's manifest-inspect view rendering rerank scores.

### 9. Observability
- `memory_rerank_duration_seconds` histogram.
- `memory_rerank_pool_size` histogram.
- `memory_rerank_skipped_total{reason="model_missing|disabled|latency_budget"}`.
- Per-retrieval span `memory.retrieve` with child spans `hybrid_search`, `rerank`, `score_blend`.

### 10. Risks and rollback
- **Latency spike.** Set `maxLatencyMs`; on first call exceeding budget, fall back to composite-only for the rest of the session and log loudly. Rollback: `enabled=false`.
- **Memory footprint.** ~150MB resident. Acceptable; monitor `RSS` in dev.
- **Miscalibration on code-identifier queries.** Detected by Tier 1 ablation; if regression, gate disabled by default.

### 11. Acceptance criteria
- With `enabled=true`, Tier 1 recall@5 ≥ baseline + 3.
- P95 retrieval latency ≤ 250ms.
- With `enabled=false`, retrieval behaves exactly as pre-B2.
- Manifest panel displays rerank column when present, omits cleanly when absent.

### 12. Open questions
- **Blend weight starting value.** Plan says 0.6. **Recommend: ship with 0.6, expose as setting, revisit after Tier 2 has scenarios.**
- **Keep `bge-reranker-v2-m3` as alternate?** Plan suggests fallback. **Recommend: not now** — adds a model-discovery code path with no concrete demand. Add when a user reports a class where modernbert reranker underperforms.
- **Apply trust multiplier before or after rerank blend?** Open question per the plan. **Recommend: blend first, then apply scope/trust multipliers** — keeps the rerank score interpretable in the manifest and preserves the trust-discipline invariant from A3. Document in B2c.

---

## B3. Remove 0.70 early-exit; merged multi-scope retrieval

### 1. Summary
Replace the cascade-with-early-exit at [store.rs:1051-1062](crates/gaviero-core/src/memory/store.rs#L1051) with parallel per-scope retrieval feeding a single merged pool, ranked by composite score (with scope multiplier as a soft bias) and reranked by B2. Fixes the "Run drowns Repo" failure mode. User-visible effect: high-quality Repo memories surface even when a marginal Run match exists. Non-goals: scope-multiplier auto-tuning, dynamic pool sizing per query.

### 2. Dependencies and ordering
- T0, B4 mandatory before merge (eval gate).
- B1 before this — ensures we tune scope multipliers against the new embedder's similarity distribution.
- B2 strongly preferred before this — without rerank, a 50-candidate merged pool is noisier than the cascade.
- `confidence_threshold` field on `SearchConfig` becomes vestigial; deprecate but keep struct field for one release with `#[deprecated]` to avoid breaking external callers.

### 3. Affected crates and modules
```
crates/gaviero-core/src/memory/
├── scoring.rs            [modify] tune scope_weight() values + tests
├── store.rs              [modify] cascading_search → multi_scope_retrieve; remove early-exit
├── retrieval.rs          [modify] retrieve_for_chat orchestrates parallel scopes
└── schema.rs             [modify] manifest scope-distribution summary field
crates/gaviero-core/src/workspace.rs                  [modify] scope multiplier settings
crates/gaviero-tui/src/panels/memory_panel.rs         [modify] scope-distribution badge
```

### 4. New types, traits, public API
- New `MemoryStore::multi_scope_retrieve(query: &str, scopes: &[ScopeFilter], per_scope_k: usize) -> Result<Vec<ScoredMemory>>`. Spawns `FuturesUnordered` of per-scope hybrid searches; awaits all; merges; dedupes by `content_hash`; sorts by composite score.
- `cascading_search` retained as a thin wrapper that calls `multi_scope_retrieve` over the same scope chain — keeps existing test coverage and any external callers.
- `ScoredMemory.scope_path` already carries scope identity; no struct change.
- Manifest payload gains `scope_distribution: Vec<(scope_label, count_in_pool, count_selected)>`.

### 5. Data-flow description
```
retrieve_for_chat(query)
  └─ FuturesUnordered:
       ├─ per_scope_search(Run)        ─┐
       ├─ per_scope_search(Module)      │
       ├─ per_scope_search(Repo)        │ all run concurrently;
       ├─ per_scope_search(Workspace)   │ each holds the SQLite lock briefly,
       └─ per_scope_search(Global)     ─┘ embedding done once upfront and shared
  └─ merge: dedup by content_hash, keep narrowest-scope occurrence
  └─ score: composite × scope_mult × trust  (B4 floor + exemptions apply)
  └─ rerank stage (B2)
  └─ select top-K under token budget
```
Embedding is computed **once** for the query and passed to each per-scope task. Each task takes the lock only for its own SQLite read; no task holds the lock across the rerank or the merge.

### 6. Schema / settings changes
- Settings (defaults):
  - `memory.scoping.runMultiplier: f32` = `1.10`
  - `memory.scoping.moduleMultiplier: f32` = `1.05`
  - `memory.scoping.repoMultiplier: f32` = `1.00`
  - `memory.scoping.workspaceMultiplier: f32` = `0.95`
  - `memory.scoping.globalMultiplier: f32` = `0.85`
  - `memory.retrieval.perScopeTopK: usize` = `20`
  - `memory.retrieval.maxMergedPool: usize` = `50` (caps the input to rerank)
- The current `scope_weight()` defaults at [scoring.rs:95-103](crates/gaviero-core/src/memory/scoring.rs#L95-L103) (1.8/1.5/1.2/1.0/0.8) are far more aggressive than the plan suggests. **B3 lowers them** to the values above. This is a tuning change with eval impact — must run Tier 1.
- Manifest payload gains `scope_distribution`.

### 7. Task breakdown
| ID | Description | Size | Deps |
|----|-------------|------|------|
| B3a | `multi_scope_retrieve`; share single query embedding across per-scope tasks | M | — |
| B3b | Remove early-exit at store.rs:1051; deprecate `SearchConfig.confidence_threshold` | S | B3a |
| B3c | Lower scope multipliers to plan defaults; settings wiring | S | B3a |
| B3d | Manifest `scope_distribution` summary; CLI manifest output formatted | S | B3a, S4 |
| B3e | Panel cross-scope badge in inject-now and inspect-manifest | S | B3d, A4 |
| B3f | **"Run drowns Repo" regression test** — fixture: Repo memory sim 0.90 + Run memory sim 0.72; assert Repo wins | S | B3a |
| B3g | **Tier 1 + Tier 2 gates** — must pass with new multipliers | M | T0, B3c |

### 8. Test strategy
- Unit: `multi_scope_retrieve` joins all five scopes; embedding computed once; dedup keeps narrowest scope.
- Integration: B3f regression test (the named "Run drowns Repo" fixture).
- Concurrent-call test: 10 simultaneous `retrieve_for_chat` calls should not deadlock or starve any scope.
- Tier 1 gate: scope-multiplier change must not regress recall@5 by >2 points on any tag.
- Tier 2 gate: at least one scenario should improve from cross-scope visibility; zero scenarios may regress.

### 9. Observability
- `memory_retrieve_per_scope_duration_seconds{scope}` histogram.
- `memory_retrieve_pool_size_total{scope}` counter.
- `memory_retrieve_dedup_drops_total` — count of cross-scope duplicates collapsed.
- Span `memory.retrieve.multi_scope` with one child span per scope.

### 10. Risks and rollback
- **Latency.** 5 parallel scope reads + larger pool + rerank = potential +50–100ms. Mitigations: per-scope top-K cap (20), merged pool cap (50), warmup, the per-scope concurrency itself overlaps with embedding.
- **Multiplier miscalibration.** Setting Global too high leaks cross-project memories. Mitigation: Global default `0.85` is conservative; gated by Tier 1.
- **Global DB I/O.** Global memory.db lives in a separate file. Ensure connection is opened once at workspace bootstrap and reused.
- **Rollback.** Settings flag `memory.retrieval.mode: "merged" | "cascade"` could gate B3a behind a kill switch. **Recommend: ship the kill switch** for the first release; remove in the next.

### 11. Acceptance criteria
- "Run drowns Repo" fixture passes.
- No 0.70 magic number anywhere in `cascading_search`/`multi_scope_retrieve` (grep gate).
- Tier 1 non-regression; Tier 2 ≥ baseline − 1 with at least one improvement.
- Manifest contains `scope_distribution` after B3d.

### 12. Open questions
- **Scope multipliers.** Plan suggests 1.10/1.05/1.00/0.95/0.85. Current code has 1.8/1.5/1.2/1.0/0.8 — much steeper. **Decision: adopt plan values**, but verify in Tier 1 before flipping default. If Tier 1 regresses, intermediate values (1.4/1.2/1.0/0.9/0.8) may be a better starting point.
- **Merged pool cap.** 50 candidates is the plan's default; with 5 scopes × 20 = 100, we drop 50 by composite ranking before rerank. Is 50 enough headroom? **Recommend: ship at 50**, monitor `pool_size_total` distribution; raise to 75 if rerank rarely changes top-10.
- **Keep `cascade` mode behind kill switch?** Yes for first release; remove after one stable cycle.

---

## Cross-cutting acceptance for Phase 1 completion

1. Eval harness exists; Tier 1 baseline checked in.
2. B4: 180-day decision outranks 7-day observation; settings-tunable; panel shows decay state.
3. B1: `gte-modernbert-base` is default after migration; nomic preserved as a fallback path; Tier 1 non-regressed.
4. B2: reranker default ON after ablation gate green; falls back silently when disabled or model missing; manifest carries `rerank_score`.
5. B3: no 0.70 anywhere in retrieval; merged pool with soft scope multipliers; "Run drowns Repo" fixture passes; manifest carries `scope_distribution`.
6. No regression in Tier S or Tier A criteria.
7. Single `retrieve_for_chat` entry point used by chat injection (S1), MCP `memory_search` (A5), and panel search (A4) — change to retrieval affects all three uniformly.

## Decisions the developer must make before starting

1. **Eval harness CI vs manual** (T0).
2. **B4: starting decay floor 0.35 vs 0.25.**
3. **B4: include `gotcha`/`lesson`/`error` in exempt list?**
4. **B1: ship code with default `nomic` and flip in follow-up PR, or default `gte-modernbert` immediately?** (Recommend: two PRs.)
5. **B1: prefix strategy** — `"search_query: "`/`"search_document: "` vs none. Decide from B1g report.
6. **B1: keep nomic in tree indefinitely or deprecate after 1 release?**
7. **B2: blend weight 0.6 vs another start.**
8. **B2: keep `bge-reranker-v2-m3` as alternate option?**
9. **B2: trust/scope multipliers applied before or after rerank blend?** (Recommend: after.)
10. **B3: ship `cascade` kill-switch?** (Recommend: yes for first release.)
11. **B3: scope multiplier values** — adopt plan's gentle values immediately or step down from current steep values?
