# gaviero-core — Architecture

Shared execution layer. All runtime logic: swarm, memory, ACP, write gate, validation, git, terminal, repo map, context planning. No UI dependencies.

---

## 1. Module Topology

```
gaviero-core/src/
├─ lib.rs               Re-exports tree-sitter types + 21 pub mod
├─ types.rs             FileScope, WriteProposal, DiffHunk, StructuralHunk,
│                       ModelTier, PrivacyLevel, NodeInfo, normalize_path
├─ workspace.rs         Workspace, settings cascade, namespace resolution
├─ session_state.rs     SessionState, TabState, PanelState, StoredConversation,
│                       ConversationIndex (+ load_session / save_session)
├─ tree_sitter.rs       LANGUAGE_REGISTRY (16 langs), enrich_hunks
├─ diff_engine.rs       compute_hunks (via `similar`)
├─ write_gate.rs        WriteGatePipeline, WriteMode, proposal lifecycle
├─ observer.rs          WriteGateObserver, AcpObserver, SwarmObserver
├─ scope_enforcer.rs    FileScope checks (thin wrapper over path_pattern)
├─ path_pattern.rs      Glob matcher + patterns_overlap — backs DSL scope-
│                       overlap validation and swarm::validation
├─ git.rs               GitRepo, WorktreeManager, GitCoordinator (git2)
├─ query_loader.rs      Tree-sitter .scm query discovery
├─ indent/              compute_indent dispatcher
│  ├─ tree_sitter.rs    Query-based indent
│  ├─ hybrid.rs         Heuristic fallback
│  └─ bracket.rs        Bracket-counting final fallback
├─ terminal/            PTY lifecycle + OSC 133
│  ├─ pty.rs            portable-pty spawn + I/O
│  └─ osc133.rs         Prompt/command boundary parsing
├─ repo_map/            File ranking + personalized PageRank
│  ├─ store.rs          GraphStore, FileNode, DirectedEdge
│  ├─ builder.rs        Graph construction from source symbols
│  ├─ graph_builder.rs  Incremental graph updates
│  └─ pagerank.rs       PageRank over agent-preferred nodes
├─ acp/                 Claude subprocess (ACP protocol)
│  ├─ session.rs        AcpSession (spawn, streaming, tempfile spill)
│  ├─ protocol.rs       NDJSON parsing, ContentDelta / ToolUse / Result
│  ├─ client.rs         AcpPipeline, propose_write, prompt enrichment
│  └─ factory.rs        AcpSessionFactory (one_shot / persistent)
├─ agent_session/       V9 transport layer — AgentSession trait + impls
│  ├─ mod.rs            Turn, TransportContext, build_turn (PlannerSelections
│  │                    → Turn), LegacyAgentSession shim
│  ├─ claude.rs         Claude AgentSession (backed by AcpPipeline)
│  ├─ codex_exec.rs     Codex via `codex exec`
│  ├─ codex_app_server.rs Codex app-server transport
│  ├─ ollama.rs         Ollama HTTP SSE transport
│  └─ registry.rs       Session registration + routing
├─ context_planner/     Bootstrap / delta / replay policy
│  ├─ mod.rs            ContextPlanner, plan() → PlannerSelections
│  ├─ types.rs          PlannerInput, MemorySelection, GraphSelection,
│  │                    ReplayPayload, ContinuityHandle, ProviderProfile
│  ├─ ledger.rs         SessionLedger, PlannerFingerprint, ContentDigest
│  └─ compaction.rs     CompactionPolicy, compact_replay, should_compact
├─ memory/              Scoped embeddings + consolidation
│  ├─ store.rs          MemoryStore (SQLite + sqlite-vec)
│  ├─ scope.rs          MemoryScope (5-level), WriteScope, Trust, MemoryType
│  ├─ scoring.rs        SearchConfig, ScoredMemory, scoring formula
│  ├─ consolidation.rs  Consolidator (triage → decay → promotion)
│  ├─ embedder.rs       Embedder trait
│  ├─ onnx_embedder.rs  OnnxEmbedder (nomic-embed-text-v1.5)
│  ├─ model_manager.rs  Model download + cache
│  └─ code_graph.rs     Code knowledge graph (impact queries)
├─ swarm/               6-phase orchestration
│  ├─ models.rs         WorkUnit, AgentManifest, SwarmResult, MergeResult
│  ├─ plan.rs           CompiledPlan, PlanNode, DependencyEdge
│  ├─ pipeline.rs       validate → execute → merge → verify → cleanup →
│  │                    consolidate
│  ├─ coordinator.rs    NLP task → CompiledPlan (Opus-powered)
│  ├─ planner.rs        Static planner utilities
│  ├─ validation.rs     Pairwise scope overlap, topological sort
│  ├─ router.rs         TierRouter: (tier, privacy) → ResolvedBackend
│  ├─ privacy.rs        PrivacyScanner (glob privacy override)
│  ├─ replanner.rs      Mid-execution replanning decisions
│  ├─ execution_state.rs Checkpoint / resume
│  ├─ merge.rs          Claude-assisted merge conflict resolution
│  ├─ bus.rs            AgentBus (broadcast + targeted)
│  ├─ board.rs          SharedBoard (tagged agent findings)
│  ├─ context.rs        Repository context collection
│  ├─ backend/          AgentBackend + shared, executor, runner,
│  │                    claude_code, ollama, codex, mock
│  └─ verify/           structural, diff_review, test_runner, combined
├─ iteration/           IterationEngine (retry, BestOfN, TDD)
└─ validation_gate/     ValidationPipeline, TreeSitterGate, CargoCheckGate
```

---

## 2. Core Data Structures

**`FileScope` (`types.rs`):** `owned_paths`, `read_only_paths`, `interface_contracts`. Matched through `path_pattern` globs. Pairwise overlap uses `path_pattern::patterns_overlap` so glob-disjoint siblings (`plans/claude-*.md` vs `plans/codex-*.md`) are accepted.

**`WorkUnit` (`swarm/models.rs`):** id, description, scope, depends_on, coordinator instructions, model / tier / privacy, retries + escalation tier, memory routing (`read_namespaces`, `write_namespace`, `memory_importance`, `staleness_sources`, `memory_read_query`, `memory_read_limit`, `memory_write_content`), context (`impact_scope`, `context_callers_of`, `context_tests_for`, `context_depth`).

**`CompiledPlan` (`swarm/plan.rs`):** `DiGraph<PlanNode, DependencyEdge>`, `max_parallel`, `iteration_config`, `verification_config`, `loop_configs`, `source_file`. Methods: `work_units_ordered`, `from_work_units`, `hash`.

**`UnifiedStreamEvent` (`swarm/backend/mod.rs`):** `TextDelta | ThinkingDelta | ToolCallStart/Delta/End | FileBlock | Usage | Error | Done`.

**`MemoryScope` (`memory/scope.rs`):** 5-level resolved scope chain (`global`, `workspace`, `repo`, `module`, `run`) with `levels()` returning narrowest→widest order. `WriteScope` is always explicit.

**`PlannerSelections` (`context_planner/types.rs`):** memory + graph + replay payloads and `ProviderProfile`. Produced by `ContextPlanner::plan`, lifted into a `Turn` by `agent_session::build_turn` (thin, lossless).

---

## 3. Swarm Pipeline (6 phases)

```
VALIDATE   pairwise scope overlap (path_pattern), Kahn topo-sort,
           dependency_tiers → Vec<Vec<WorkUnit>>
EXECUTE    per tier: for each unit (Semaphore-bounded parallel):
             git worktree checkout (gaviero/{unit_id})
             IterationEngine::run
               attempts loop with escalation:
                 build_prompt (memory + graph + board + feedback)
                 backend.stream_completion → UnifiedStreamEvent
                 each FileBlock → write_gate.insert_proposal
                 ValidationPipeline::run → corrective retry on fail
             Checkpoint ExecutionState
MERGE      git merge --no-ff main; MergeResolver (Claude) on conflict
VERIFY     StructuralOnly | DiffReview | TestSuite | Combined (early-exit)
CLEANUP    WorktreeManager::teardown_all, drop gaviero/* branches
CONSOLIDATE triage (importance ≥ 0.4) → decay (30-day half-life) →
            cross-scope promotion (3+ module hits → repo, ×1.2)
```

---

## 4. Memory

**Scope hierarchy (narrow→wide):** `Run → Module → Repo → Workspace → Global`.

**Search (`store.rs::search_scoped`):**
```
1. NO LOCK   embedder.embed(query) [ONNX]
2. CASCADE   for each level narrowest→widest:
               BRIEF LOCK vec_search_at_level
               BRIEF LOCK fts_search_at_level (optional)
               NO LOCK    merge_rrf(vec 0.7, fts 0.3)
               NO LOCK    score = (sim*0.5 + importance*0.2 +
                                   recency*0.15 + 0.15) * scope * trust
               EXIT       if best_score > 0.70
3. NO LOCK   deduplicate by content_hash
4. return top-K ScoredMemory
```

**Write (`store_scoped`):** embed → SHA-256 content hash → brief-lock dedup (reinforce / skip-because-broader / insert) → brief-lock insert into `vec_memories_scoped` + `memories_fts`.

**Consolidation (`consolidation.rs`):** `triage` promotes `importance ≥ 0.4` run memories to Module/Repo and deletes run rows; `decay_and_prune` applies `exp(-0.023 * days)`; `cross-scope promotion` promotes items accessed from ≥ 3 distinct modules to repo scope with a 1.2× importance boost.

**Embedder:** `OnnxEmbedder` — nomic-embed-text-v1.5 (768 dim), `ort 2.0` + `tokenizers`, mean pooling + L2 norm, cosine distance via `sqlite-vec`.

---

## 5. Backend Abstraction

`AgentBackend` trait produces `Stream<Item = Result<UnifiedStreamEvent>>`. Implementations: `ClaudeCodeBackend` (ACP subprocess), `OllamaStreamBackend` (HTTP SSE), `CodexBackend` (OpenAI-compatible), `MockBackend`.

**Model spec (`backend/shared.rs::backend_config_for_model`):**
```
sonnet | opus | haiku     → claude:<same>
claude:<name>             → Claude API
ollama:<name>             → Ollama
codex:<name>              → Codex
local:<url>               → OpenAI-compatible endpoint
```

**Tier routing (`router.rs::TierRouter`):** `(ModelTier, PrivacyLevel) → ResolvedBackend`.

**Privacy override (`privacy.rs::PrivacyScanner`):** glob match against `**/*.key`, `**/.env`, … promotes a unit to `LocalOnly` regardless of declared level.

---

## 6. Context Planner + Agent Session (V9)

```
PlannerInput  ──► ContextPlanner::plan ──►  PlannerSelections
                                              │
                                              ▼
                                    agent_session::build_turn
                                              │
                                              ▼
                                              Turn
                                              │
                                              ▼
                         impl AgentSession::send_turn(Turn)
                                  (Claude / Codex / Ollama)
                                              │
                                              ▼
                           Stream<UnifiedStreamEvent>
```

The planner is the single owner of memory queries, graph selection, replay, and continuity. `LegacyAgentSession` wraps `AcpPipeline` for byte-identical migration; provider-specific impls replace it progressively.

---

## 7. Write Gate

Modes: `Interactive` (queue → TUI review), `AutoAccept` (validate + write), `Deferred` (batch), `RejectAll` (drop silently).

Lifecycle:

```
write(path, content)
  ├─ BRIEF LOCK   is_scope_allowed(agent_id, path)  [path_pattern]
  ├─ NO LOCK      compute_hunks → enrich_hunks (StructuralHunk)
  ├─ BRIEF LOCK   insert_proposal (mode-specific)
  └─ NO LOCK      fs::write if finalized
```

Observers: `on_proposal_created / updated / finalized`. UI can accept/reject per hunk or per AST node (`enclosing_node`).

---

## 8. ACP (Claude subprocess)

`AcpSessionFactory` spawns `one_shot` or `persistent` sessions. `AcpSession` handles NDJSON over stdin/stdout; large prompts spill to a tempfile (`ARGV_THRESHOLD` in `session.rs`). Events: `SystemInit`, `ContentDelta`, `ToolUseStart`, `AssistantMessage`, `ResultEvent` (parsed in `protocol.rs`). `AcpPipeline` enriches prompts and routes detected file blocks through the write gate.

---

## 9. Concurrency

| Component | Primitive | Rule |
|---|---|---|
| `WriteGatePipeline` | `tokio::sync::Mutex` | No lock across diff, parse, I/O |
| `MemoryStore` | `tokio::sync::Mutex<rusqlite::Connection>` | Embed outside lock |
| `ExecutionState` | `Mutex<Vec<NodeStatus>>` | Checkpoint after each node |
| `AgentBus` | `broadcast::channel` | Lock-free |
| Parallel agents | `Semaphore` | Bounded per tier |

**Never hold a Mutex across `await`, tree-sitter parse, or `fs` I/O.**

---

## 10. Error Handling

| Error | Recoverable | Handling |
|---|---|---|
| parse/compile | compile-time | miette diagnostic with span |
| scope violation | no | reject proposal, observer, log |
| agent failure | yes | `AgentStatus::Failed`, escalate if configured |
| validation gate | yes | corrective feedback → retry |
| merge conflict | yes | Claude resolution or user choice |
| memory init | no, non-fatal | `Option<Arc<MemoryStore>>`, continue |
| consolidation / cleanup | no, non-fatal | log, continue |

---

## 11. Hard Constraints

1. All agent writes through `WriteGatePipeline`.
2. `git2` only.
3. Tree-sitter for syntax, highlight, indent.
4. `MemoryStore` behind `tokio::sync::Mutex`; embedding outside lock.
5. No UI types in core.
6. `AgentBackend` + `UnifiedStreamEvent` provider-agnostic; selection via `TierRouter` + `PrivacyScanner`.
7. Explicit `WriteScope` — never inferred.
8. Swarm branches `gaviero/{work_unit_id}`; worktrees `.gaviero/worktrees/{id}/`, cleanup via `Drop`.

---

## 12. Public API

```rust
pub mod acp;              pub mod agent_session;   pub mod context_planner;
pub mod memory;           pub mod swarm;           pub mod repo_map;
pub mod write_gate;       pub mod validation_gate; pub mod iteration;
pub mod workspace;        pub mod git;             pub mod session_state;
pub mod path_pattern;     pub mod scope_enforcer;  pub mod observer;
pub mod types;            pub mod tree_sitter;     pub mod diff_engine;
pub mod query_loader;     pub mod indent;          pub mod terminal;

pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};
```

---

See [CLAUDE.md](CLAUDE.md) for conventions, build, and rules.
