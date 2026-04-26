# Tier C Part 1 — Graph and Pluggable Traits

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, schemas, migrations, task ordering, and acceptance criteria. Flag decisions that require human judgment. Do not produce full Rust code listings — high-level signatures, type skeletons, and data-flow descriptions are acceptable.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, petgraph, and graph algorithms (PageRank, personalized PageRank, IDF). Do not explain these.

**This document covers Phase 1 of Tier C**, containing three items (C5, C3, C4) focused on the code graph and the pluggability of embedder/reranker traits. Phase 2 of Tier C (C1 typed memory stores, C2 `/forget` with audit trail) is a separate document and is **storage-focused**. The two phases are independent — each can ship without the other, and C1+C2 in Phase 2 should run after this phase lands only because C5's trait formalization is easier to build on when the schema work (C1) isn't also in flight.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (ratatui TUI editor) and `gaviero-cli` (headless runner). It orchestrates Claude Code (ACP), Codex, Ollama, and mock backends through a provider-agnostic `AgentBackend`. Agent writes flow through a `WriteGatePipeline`. Swarm orchestration runs a 6-phase pipeline.

Local-first, single-developer-oriented.

### Stack

- Rust 2024, tokio, petgraph, ropey, git2, ratatui 0.30
- SQLite (WAL mode) + sqlite-vec
- ONNX Runtime (`ort`) with `gte-modernbert-base` embedder (Tier B1) + `gte-reranker-modernbert-base` reranker (Tier B2)
- tree-sitter 0.25, 16-language registry
- logos + chumsky + miette for DSL

### Crate layout

- `gaviero-core` (lib) — all runtime, no UI
- `gaviero-tui`, `gaviero-cli`, `gaviero-dsl`, `tree-sitter-gaviero`

### Memory subsystem state (assuming Tier S, A, B are in place)

- 5-level scope: `Global → Workspace → Repo → Module → Run`
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- **Merged multi-scope retrieval** (Tier B3) — no early-exit; scope bias via multipliers
- **Cross-encoder reranker** in retrieval pipeline (Tier B2)
- **`gte-modernbert-base` embedder** (Tier B1)
- Composite score: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust`, blended with reranker
- **Decay floor at 0.35; type-based exemptions** for decision/convention/invariant/preference (Tier B4)
- SHA-256 + semantic near-dup dedup on writes
- **Writer task owns all writes** (Tier S2)
- **Retrieval manifest persistence** — every injection produces an `injection_manifests` row (Tier S4)
- **Three-cadence consolidation**: per-turn extractor (S3), per-session consolidator (B5), sleeptime pass (B5)
- **Retrieval-use telemetry** feeding sleeptime trust re-scoring (Tier B6)
- **Annotations sidecar** parsed from LLM output (Tier A1)
- **`/remember` scope-corrected and visible** (Tier A2)
- **`source` and `trust` attributes** populate on all writes (Tier A3)
- **TUI memory panel** for inspection and curation with bulk delete, pin, scope-change, manifest inspection (Tier A4)
- **Gaviero-as-MCP-server** with three read-only tools: `memory_search`, `blast_radius`, `node_doc` (Tier A5)
- `Embedder` and `Reranker` traits with ONNX impls; model files under `~/.cache/gaviero/models/`

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle. Phase 1 of Tier C doesn't touch the memory schema directly — it touches the trait boundaries around retrieval components (C5) and the knowledge-graph data structures (C3, C4) that support the `blast_radius` MCP tool and the Tier D1 node-doc system to come.

### Knowledge graph / repo-map state

- `repo_map/` has `GraphStore`, `FileNode`, `DirectedEdge`
- Graph is built from tree-sitter-extracted symbols
- **Currently untyped edges** — all edges treated equivalently in PageRank (this is what C4 changes)
- Personalized PageRank seeded by agent-preferred nodes computes relevance
- Used for `impact_scope` blast-radius, `callers_of`, `tests_for`, `depth`-bounded queries; surfaced via MCP `blast_radius` tool (Tier A5)

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

- Writer task architecture; read-only MCP; annotations sidecar; three-cadence consolidation.
- No LLM writes; no graph-RAG / Neo4j / A-MEM memory evolution.
- `gte-modernbert-base` is the embedder; cross-encoder reranker enabled.
- Pluggability for embedder/reranker is partially realized for the specific models shipped in Tier B; **C5 in this phase completes it as a general facility.**

---

## Phase 1 context and rationale

Phase 1 of Tier C delivers **structural refinements** around retrieval pluggability and the code graph. Each item is long-lead investment in precision and maintainability, justifiable on its own merits but lower-ROI than any item in S/A/B in terms of immediate user-visible quality improvement.

- **C5 (pluggable embedder and reranker traits).** Generalize what Tier B made possible for specific models into a clean trait-based extensibility surface. Ship first because it formalizes what Tier B introduced; easier to stabilize now than later. Also unblocks C6's dual-embedder A/B capability for future model evaluations.
- **C3 (HippoRAG-style node-specificity in PageRank).** IDF-like downweighting of generic symbols. Free precision gain on blast-radius queries.
- **C4 (typed edges in code graph).** `CALLS | IMPORTS | IMPLEMENTS | DEFINES | TEST_OF | REFERENCES_DOCSTRING_OF | DECLARES_CONTRACT_WITH`. Per-query-intent edge weighting. Supports the MCP `blast_radius` tool's future `mode` parameter.

**Dependencies and ordering within Phase 1:**

- **C3 and C4 are independent** and can ship in any order; can run in parallel if staffing allows.
- **C5 should ship first.** It formalizes the trait boundaries without changing behavior; lands cleanly without other concurrent changes.
- **C3 and C4 both touch the graph subsystem** but don't conflict: C3 adds a `specificity_map`; C4 adds an `EdgeKind` enum. Both sub-systems can be developed concurrently.

**Recommended sequence:** C5 → (C3 ∥ C4). C3 and C4 can interleave if a single developer is working on both.

**All Phase 1 items must continue to satisfy the evaluation gates from Tier B.** Tier 1 retrieval smoke test runs pre/post; no regression allowed. C3 and C4 additionally need blast-radius-specific regression tests — pinned `(changed_file, expected_impacted_files)` pairs.

---

## Items in this phase

### C5. Pluggable embedder and reranker traits

**Problem.** Tier B1 and B2 introduced specific models (gte-modernbert for embedding, gte-reranker-modernbert for reranking) with implementation-specific code paths. The abstraction is partially there but not fully generalized. The embedding landscape shifts every 6 months; a principled trait-based plugin system lets future model swaps be non-invasive and enables A/B testing.

**Architecture.**

- **Formalize `Embedder` trait** in `gaviero-core/src/memory/embedder.rs`:
  ```
  #[async_trait]
  trait Embedder: Send + Sync {
      fn name(&self) -> &'static str;
      fn dimension(&self) -> usize;
      fn max_tokens(&self) -> usize;
      async fn embed(&self, text: &str, purpose: EmbeddingPurpose) -> Result<Vec<f32>>;
      async fn embed_batch(&self, texts: &[&str], purpose: EmbeddingPurpose) -> Result<Vec<Vec<f32>>>;
  }
  
  enum EmbeddingPurpose { Query, Document }
  ```
- **Formalize `Reranker` trait** in `gaviero-core/src/memory/reranker.rs`:
  ```
  #[async_trait]
  trait Reranker: Send + Sync {
      fn name(&self) -> &'static str;
      fn max_tokens(&self) -> usize;
      async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>>;
  }
  ```
- **Implementations:**
  - `OnnxEmbedder<M: EmbeddingModel>` — generic over a type marker for the model family. Current impls: `NomicV15`, `GteModernBertBase`. Tokenizer + ONNX session loaded from the model manager.
  - `OnnxReranker<M: RerankerModel>` — generic over a type marker. Current impls: `GteRerankerModernBertBase`, optional `BgeRerankerV2M3`.
  - `NullEmbedder` / `NullReranker` for testing (returns random vectors / identity rerank).
  - `ApiEmbedder<P: ApiProvider>` — behind a feature flag `--features api-embedders`, implements Voyage, Cohere, OpenAI. **Disabled by default; local-first is the priority.**
- **Runtime selection:** settings resolves embedder/reranker name at startup, loads the corresponding impl. No dynamic loading at plugin boundaries — just a match statement in a factory function.
- **A/B testing support:** a `DualEmbedder` wrapper that embeds with both a primary and a comparison embedder, logs results for offline comparison, returns the primary. Used to evaluate candidate upgrades without migration.
- **Dimension mismatch handling:** changing embedders changes vector dimensions (e.g., 768 → 1024). Schema has `embedding_dim` column; retrieval filters by matching dim. During migration, coexistence is supported (each row's vector is only used for queries with matching dim).

**Benefits.**

- Embedder/reranker upgrades become non-invasive config changes.
- Enables offline A/B testing before migration.
- Makes the memory system testable with mock embedders/rerankers.
- Prepares for future: Qwen3 embeddings, other local models, Voyage/Cohere for power users willing to leave local-first.

**Risks.**

- **Over-abstraction.** Traits with too many methods or overly generic signatures become painful. Keep the trait minimal; resist baroque capabilities.
- **Batch API contract.** `embed_batch` must be efficient (single ONNX session call); naive impls would loop. Document the contract.
- **Dimension mismatch coexistence.** During upgrade transitions, old rows and new rows have different dims. Retrieval must handle both gracefully or force full migration before allowing new writes.

**Alternatives considered and rejected.**

- *Hardcode the current model set.* Fine until upgrade.
- *External service for embeddings.* Violates local-first.
- *Dynamic library loading (dylib plugins).* Adds OS-specific complexity, security surface. Not worth it.

**Integration notes.**

- The factory function (`create_embedder_from_settings`, `create_reranker_from_settings`) is the single place to add new implementations. Document the pattern.
- The model manager (`model_manager.rs`) continues to handle download/cache. Each new model adds a download URL + checksum.
- Settings:
  - `memory.embedder.name: string` (values: `"nomic-v15" | "gte-modernbert" | "null" | "voyage-code-3" | ...`)
  - `memory.reranker.name: string` (values: `"gte-reranker-modernbert" | "bge-reranker-v2-m3" | "null" | "none"`)
  - `memory.embedder.apiKey: Option<string>` (only for API-based impls; respect env var override)
- Tests: every trait impl passes a shared test battery (embeds known-similar texts higher than known-different, handles empty input, handles very-long input by truncating, batch produces same results as single, etc.).
- B6 retrieval-use telemetry (Tier B Phase 2) uses the `Embedder` trait to embed responses. C5's formalization means B6 works against any conforming embedder.

---

### C3. HippoRAG-style node-specificity weighting in PageRank

**Problem.** Personalized PageRank on the code graph treats all symbols equivalently, but generic symbols (`Option`, `Result`, `String`, `Vec`, `println!`, `clone`, `new`) appear in nearly every file and create near-fully-connected graph regions. They attract disproportionate PageRank mass, diluting the signal for specific domain symbols. Aider's original algorithm shares this issue.

**Architecture.**

- Compute a **node-specificity score** per symbol at graph-build time:
  - `specificity(symbol) = log(N / df(symbol))` where `N` is total files, `df(symbol)` is files that reference the symbol (IDF).
  - Normalize to [0.0, 1.0] across the graph.
  - Symbols referenced in >50% of files (stop-symbols) get specificity ≈ 0.
  - Rare domain symbols get specificity ≈ 1.0.
- **Edge weighting by specificity:** when PageRank propagates through an edge, multiply the propagation weight by the target symbol's specificity. Generic symbols receive PageRank but propagate it weakly (their specificity is low); rare symbols propagate strongly.
- Implementation: `repo_map/pagerank.rs` takes an additional `specificity: &HashMap<NodeId, f32>` parameter. The PageRank iteration multiplies each transition probability by the target specificity before row-normalizing.
- Specificity is computed once at full graph-build time and updated incrementally when new symbols are added (via `graph_builder::incremental_update`).
- Cached on the `GraphStore` struct; recomputed when global `df` estimate drifts >10%.

**Benefits.**

- Free precision gain on `impact_scope` / `blast_radius` queries. No LLM cost. Deterministic. Interpretable.
- Addresses a known Aider failure mode on large stdlib-heavy codebases.
- Ships with a visible specificity indicator in the TUI panel's blast-radius view.

**Risks.**

- **Miscalibration cutoff.** If the 50% stop-symbol cutoff is wrong for a given codebase, either over-downweights real cross-cutting symbols or doesn't help. Make it tunable.
- **Rare but common-in-production symbols.** A symbol rare in test corpus but common in production logic could be misweighted. Mitigate by computing specificity over the actual repo, not a corpus.
- **Incremental update complexity.** Adding a new symbol may change specificity of many symbols. Acceptable as long as full recomputation is <2 seconds on a medium repo.

**Alternatives considered and rejected.**

- *Manual stop-symbol list.* Unmaintainable at scale; per-language and per-codebase differences.
- *TF-IDF-weighted edges (bidirectional).* More signal, more computation. Start with IDF-weighted targets only; extend later if needed.
- *Do nothing (current).* Works adequately for small repos; degrades on large stdlib-heavy ones.

**Integration notes.**

- The `GraphStore` gains a `specificity_map: HashMap<NodeId, f32>`. Serialized alongside the rest of the graph.
- Settings:
  - `repoMap.specificity.enabled: bool` (default true)
  - `repoMap.specificity.stopSymbolThreshold: f32` (default 0.5, meaning referenced in >50% of files)
- TUI integration: when the panel or chat displays a blast-radius result, show specificity as a small badge next to each file (e.g., `sp 0.92`).
- Test: construct a fixture graph with a known stop-symbol and verify it receives low PageRank mass after specificity weighting.
- Because blast-radius is exposed via MCP (Tier A5), specificity-weighted results automatically flow to subprocess agents.

---

### C4. Typed edges in the code graph

**Problem.** Current graph edges are undifferentiated (`DirectedEdge` doesn't carry a type). Every edge contributes equivalently to PageRank. But different query intents should weight edges differently:

- "Who calls this function?" → `CALLS` edges only.
- "Who implements this trait?" → `IMPLEMENTS` edges only.
- "What tests exist for this module?" → `TEST_OF` edges only.
- "Blast radius of changing this type" → `CALLS` + `IMPLEMENTS` + `DEFINES` weighted heavily; `REFERENCES_DOCSTRING_OF` weighted lightly.

**Architecture.**

- Introduce an `EdgeKind` enum:
  ```
  enum EdgeKind {
      Calls,           // fn A calls fn B
      Imports,         // module M imports N
      Implements,      // type T implements trait U
      Defines,         // module M defines symbol S
      TestOf,          // test file T tests symbol/file S
      ReferencesDocstringOf, // docstring of X references Y
      DeclaresContractWith,  // (from Tier D1 node docs; edge promoted from text)
  }
  ```
- Extend `DirectedEdge` with `kind: EdgeKind`. Schema migration: existing edges default to `Calls` (best guess) or `Imports` (more neutral) — recommend `Imports`. Ideally, rebuild the graph on first run after migration using the new typed extraction.
- Tree-sitter queries in `query_loader.rs` extract typed edges by query type. Update per-language `.scm` query files to distinguish node types producing different edge types.
- PageRank operates on typed graphs: `pagerank_typed(graph, edge_weights: HashMap<EdgeKind, f32>)`. Different query intents produce different weight maps.
- MCP `blast_radius` tool (Tier A5) gains a `mode` parameter: `impact | callers | tests | implementations | all`. Each mode selects an edge-weight preset:
  - `impact`: Calls=1.0, Implements=0.9, Defines=0.8, TestOf=0.3, Imports=0.5, Other=0.2
  - `callers`: Calls=1.0, others=0.0
  - `tests`: TestOf=1.0, others=0.0
  - `implementations`: Implements=1.0, others=0.0
  - `all`: all=1.0 (equivalent to current behavior)
- Rebuild strategy: full graph rebuild is fast (<30s on a medium repo); do it once on migration. Incremental builds use the per-language typed extraction going forward.

**Benefits.**

- Blast-radius queries become much more precise.
- Opens future IDE interop (SCIP/LSIF model edges as typed).
- Makes the graph useful for question types beyond "what's nearby."
- Each edge type weighted per intent → one graph serves many query patterns.

**Risks.**

- **Tree-sitter query complexity.** Per-language `.scm` files gain edge-kind-producing queries. More code to maintain across 16 languages. Start with Rust and TypeScript; extend as needed.
- **Edge explosion.** Typed edges may mean more edges if one logical relationship is now 2–3 typed edges. Monitor graph size.
- **Edge-weight presets are tuning knobs.** Empirical; tune against Tier 2 eval scenarios.
- **Migration requires full graph rebuild.** 30-second stall on large repos. Show a progress indicator.

**Alternatives considered and rejected.**

- *Stay with untyped edges.* Simpler but caps query precision.
- *Full SCIP/LSIF schema.* Overkill for Gaviero's scope; adds heavy dependencies.
- *Lazy typing on query.* Re-parse on every query; too expensive.

**Integration notes.**

- The 16 tree-sitter languages have varying support levels. Plan should prioritize: Rust, TypeScript, Python, Go first; others receive `Imports` + `Defines` as a minimum.
- The per-language query file updates are a significant sub-task; treat as its own multi-part task.
- TUI blast-radius panel shows edge types on the paths between nodes.
- Settings:
  - `repoMap.edges.typed: bool` (default true after migration)
  - `repoMap.edges.weights.<intent>` — per-intent weight maps, user-override-able.
- MCP `blast_radius` tool schema (Tier A5) gains `mode` parameter — this is a visible MCP surface change; update the tool schema docs and call out to subprocess-agent-aware users.
- C4's `DeclaresContractWith` edge kind is a placeholder for Tier D1; until D1 ships, it is never populated. No behavior change from its presence in the enum.

---

## Expected output format from the implementation plan

For each of C5, C3, C4 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from S, A, B, or earlier C items); sub-task order.

**3. Affected crates and modules.** Tree-style.

**4. New types, traits, public API.** High-level signatures. For C4 specifically: the MCP `blast_radius` tool schema update.

**5. Data-flow description.** For C3: graph-build-time specificity computation → PageRank-time edge weighting. For C4: tree-sitter extraction → typed edge storage → intent-weighted PageRank → MCP tool response shaping.

**6. Schema / settings / UI changes.** SQL migrations (especially C4); settings.json additions; TUI changes (specificity badges, edge-type display).

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies. C4 specifically: per-language tree-sitter query updates as distinct sub-tasks.

**8. Test strategy.** Unit + integration + **evaluation gate (Tier 1 non-regression)**. C3 and C4 specifically need blast-radius regression tests — pinned `(changed_file, expected_impacted_files)` pairs; failure = regression in recall@3 on blast-radius queries. C5 requires the shared trait test battery.

**9. Observability.** Tracing, metrics, observer events. For C4: per-intent PageRank latency and edge-type distribution metrics.

**10. Risks and rollback.** Migration strategies; graph-rebuild checkpoints; revert paths. For C4 specifically: what happens if tree-sitter extraction fails for a language post-migration (fall back to untyped edges for that language; log).

**11. Acceptance criteria.** Measurable, human-testable.

**12. Open questions.** Decisions requiring developer judgment. Examples: specificity cutoff for C3; whether to force full graph rebuild on C4 migration or do incremental with some edges defaulted; which tree-sitter languages to prioritize for C4 typed extraction; whether C5's `DualEmbedder` should persist comparison results to disk or log-only.

---

## Evaluation gates

**Every Phase 1 change must be gated by Tier 1 retrieval smoke test (non-regression).** C3 and C4 additionally need blast-radius-specific regression tests — pinned `(changed_file, expected_impacted_files)` pairs. Failure = regression in recall@3 on blast-radius queries.

C5 requires the shared trait test battery to pass for every registered implementation.

---

## Explicit non-goals for Phase 1

Do **not** plan the following here:

- Typed memory stores split (Records / History / Summaries) — Phase 2 C1
- `/forget` command with audit trail — Phase 2 C2
- KG node-doc schema — Tier D1
- Contextual Retrieval for docs — Tier D2
- AGENTS.md / CLAUDE.md compat — Tier D3
- New embedders beyond what C5 enables (but doesn't implement) — future
- Full SCIP/LSIF schema — explicitly rejected
- Graph-RAG with LLM-extracted entities — explicitly rejected
- Learned node embeddings (GraphSAGE, node2vec) — explicitly rejected
- Multi-graph per workspace — not needed

---

## Acceptance criteria for Phase 1

Phase 1 is complete when all of the following are true:

1. `Embedder` and `Reranker` traits are formalized; at minimum `nomic-v15`, `gte-modernbert`, `null` (embedders) and `gte-reranker-modernbert`, `none`, `null` (rerankers) are implementations. Dual-embedder A/B mode works. Dimension mismatches are handled during transitions (coexistence supported).
2. PageRank on the code graph uses specificity weighting; stop-symbols (>50% file reference) receive effectively zero propagation. Blast-radius precision improves measurably on eval scenarios. Specificity badges visible in the TUI panel.
3. Code graph edges are typed (Calls, Imports, Implements, Defines, TestOf, ReferencesDocstringOf, DeclaresContractWith as a placeholder for D1). MCP `blast_radius` tool accepts a `mode` parameter and returns different results per mode. Per-intent weight maps are configurable. At minimum Rust, TypeScript, Python, Go have proper typed extraction; remaining languages have `Imports` + `Defines` minimum.
4. All Tier 1 retrieval smoke tests non-regress; Tier 2 code-specific tests show equal or improved scores on blast-radius-involving scenarios.
5. No regression in Tier S, A, or B acceptance criteria.

---

## Anti-patterns to avoid

- **Over-abstracting the traits.** Minimal, opinionated. One async method for the main operation, one for batch. Not a generic framework.
- **Dynamic plugin loading.** Static dispatch via factory; no dylib loading.
- **Forgetting to update tree-sitter queries for all 16 languages in C4.** Prioritize; document coverage explicitly. Missing-language fallback to untyped is acceptable; silent breakage is not.
- **Breaking the MCP `blast_radius` tool schema in a way that disrupts existing subprocess callers.** If adding the `mode` parameter, make it optional with `"all"` as the backward-compatible default.
- **Splitting C5's traits across too many files.** One trait, one file; one factory per trait.

---

## Final instruction

Produce the implementation plan per **Expected output format** above, covering C5, C3, C4 in the recommended order (C5 → C3 ∥ C4). Call out migration strategy explicitly for C4. Flag every decision the developer must make. If ambiguous, add to open questions.
