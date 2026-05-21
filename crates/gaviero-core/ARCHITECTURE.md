# gaviero-core — Architecture

Shared execution layer. All runtime logic: swarm orchestration, memory, MCP server, ACP, agent-session transport, write gate, validation, git, terminal, repo-map, context planning. **No UI or DSL dependencies.**

---

## 1. Module Topology (22 pub mods — [`src/lib.rs`](src/lib.rs))

```
gaviero-core/src/
├─ lib.rs                Re-exports tree-sitter types + 22 pub mods
├─ types.rs              FileScope, WriteProposal, DiffHunk, StructuralHunk,
│                        ModelTier, PrivacyLevel, NodeInfo, normalize_path
├─ workspace.rs          Workspace (single_folder / load), settings cascade,
│                        namespace resolution
├─ session_state.rs      SessionState, TabState, PanelState, StoredConversation,
│                        ConversationIndex (+ load_session / save_session)
├─ tree_sitter.rs        LANGUAGE_REGISTRY (16 langs), enrich_hunks
├─ diff_engine.rs        compute_hunks (similar)
├─ write_gate.rs         WriteGatePipeline, WriteMode, proposal lifecycle
├─ observer.rs           WriteGateObserver, AcpObserver, SwarmObserver
│                        (incl. on_cursor_session_started)
├─ scope_enforcer.rs     FileScope checks (delegates to path_pattern)
├─ path_pattern.rs       Glob matcher + patterns_overlap — backs DSL
│                        scope-overlap validation and swarm::validation
├─ git.rs                GitRepo, WorktreeManager, GitCoordinator (git2)
├─ query_loader.rs       Tree-sitter .scm query discovery
├─ indent/               compute_indent dispatcher (8 files: bracket /
│  ├─ bracket.rs         captures / config / heuristic / mod / predicates /
│  ├─ captures.rs        treesitter / utils). Tree-sitter query → hybrid
│  ├─ config.rs          (bracket+heuristic) → bracket-only fallback
│  ├─ heuristic.rs
│  ├─ predicates.rs
│  ├─ treesitter.rs
│  └─ utils.rs
├─ terminal/             PTY (portable-pty) + OSC 133 (12 files: config /
│  ├─ config.rs          context / event / history / instance / manager /
│  ├─ context.rs         osc / pty / session / shell_integration / types)
│  ├─ event.rs
│  ├─ history.rs
│  ├─ instance.rs
│  ├─ manager.rs
│  ├─ osc.rs
│  ├─ pty.rs
│  ├─ session.rs
│  ├─ shell_integration.rs
│  └─ types.rs
├─ repo_map/             Code knowledge graph + ranking
│  ├─ builder.rs         walks tree-sitter graph, builds typed edges
│  ├─ edges.rs           EdgeKind (Calls / Imports / Tests / Implements)
│  ├─ graph_builder.rs   SpecificityConfig
│  ├─ page_rank.rs       personalized PageRank, EdgeWeights, BlastRadiusMode
│  ├─ store.rs           persisted graph in `.gaviero/code_graph.db`
│  └─ topology.rs        Shallow filesystem map for <repo_topology>
│                        injection (TopologyConfig, build_folder_topology)
├─ acp/                  Claude subprocess (legacy ACP transport)
│  ├─ session.rs         AcpSession (spawn, NDJSON streaming, tempfile spill)
│  ├─ protocol.rs        NDJSON parsing, ContentDelta / ToolUse / Result
│  ├─ client.rs          AcpPipeline, propose_write, prompt enrichment
│  └─ factory.rs         AcpSessionFactory (one_shot / persistent)
├─ agent_session/        V9 transport — AgentSession trait + impls
│  ├─ mod.rs             Turn, TransportContext, build_turn, LegacyAgentSession
│  ├─ claude.rs          ClaudeSession (M6, ProcessBound or Stateless)
│  ├─ codex_exec.rs      CodexExecSession (StatelessReplay)
│  ├─ codex_app_server.rs CodexAppServerSession (ProcessBound, JSON-RPC 2.0)
│  ├─ cursor.rs          CursorSession (NativeResume via `agent --resume`)
│  ├─ ollama.rs          OllamaSession (StatelessReplay, HTTP SSE)
│  └─ registry.rs        SessionConstruction routing by ProviderProfile
├─ context_planner/      Bootstrap / delta / replay policy
│  ├─ mod.rs             ContextPlanner, plan() → PlannerSelections
│  ├─ types.rs           PlannerInput, MemorySelection, GraphSelection,
│  │                     ReplayPayload, ContinuityHandle (incl. CursorThreadId),
│  │                     ProviderProfile, FileAttachment, RuntimeConfig
│  ├─ ledger.rs          SessionLedger, PlannerFingerprint, ContentDigest,
│  │                     CompactionRecord, GraphDecision
│  ├─ compaction.rs      CompactionPolicy, compact_replay, should_compact
│  └─ chat_memory.rs     ChatMemoryRequest / Outcome, perform_injection,
│                        enqueue_post_turn (S3 extractor glue)
├─ mcp/                  In-process MCP server (Tier A)
│  ├─ mod.rs             public re-exports
│  ├─ server.rs          GavieroMcpServer (rmcp), spawn_mcp_server,
│  │                     McpServerHandle, cached GraphStore
│  ├─ tools.rs           memory_search / blast_radius / node_doc input/output
│  ├─ config_synth.rs    McpConfigSynth, TrustConsent;
│  │                     claude_mcp_config_json, codex_mcp_config_toml,
│  │                     cursor_mcp_config_json (same schema as Claude)
│  ├─ external_memory.rs detect / disable competing memory MCP servers
│  └─ observer.rs        McpToolCallObserver (audit log)
├─ memory/               Multi-DB scoped memory + three-cadence consolidation
│  ├─ mod.rs             init / init_workspace / init_workspace_stores;
│  │                     create_embedder_from_settings (dual: a/b supported)
│  ├─ scope.rs           MemoryScope, WriteScope, Trust, MemoryType, StoreKind
│  ├─ scoring.rs         SearchConfig, ScoredMemory, scoring formula
│  ├─ schema.rs          SQLite migrations; v10 typed-stores split (C1)
│  ├─ stores.rs          MemoryStores (multi-DB registry), search_scoped fan-out
│  ├─ store/             MemoryStore — split monolith (~7K → submodules)
│  │  ├─ mod.rs          struct + Connection ownership; probe_c1_migration
│  │  ├─ search.rs       merged + cascade retrieval
│  │  ├─ search_legacy.rs cascade kill-switch implementation
│  │  ├─ write.rs        embed → dedup → insert
│  │  ├─ panel_ops.rs    TUI memory-panel queries
│  │  ├─ deletions_ops.rs soft-delete + restore
│  │  ├─ compression_ops.rs zstd round-trip on aged History rows
│  │  ├─ sleeptime_ops.rs decay sweep, near-dup merge, promotion
│  │  ├─ telemetry_ops.rs utilization / classification queries
│  │  └─ manifest.rs     injection_manifests row writer (S4)
│  ├─ embedder.rs        Embedder trait, NullEmbedder, DualEmbedder
│  ├─ onnx_embedder.rs   OnnxEmbedder (ort 2.0 + tokenizers, mean pool + L2)
│  ├─ model_manager.rs   resolve_embedder_model + cache (gte-modernbert default)
│  ├─ reranker.rs        Optional cross-encoder; sigmoid_calibrate, blend_rerank
│  ├─ retrieval.rs       retrieve_ranked / retrieve_for_chat — single funnel
│  ├─ writer.rs          spawn_writer_task, WriterHandle, WriterMessage,
│  │                     ACK_TIMEOUT_MS=500ms (single-consumer)
│  ├─ extractor.rs       Per-turn S3 extractor (signal classification)
│  ├─ session_consolidator.rs Per-session B5 consolidator
│  ├─ consolidation.rs   Consolidator (per-run + per-session triage)
│  ├─ consolidation_llm.rs ConsolidationLlm trait + backend impl
│  ├─ sleeptime.rs       run_sleeptime (idle/weekly hygiene pass)
│  ├─ sleeptime_scheduler.rs SleeptimeScheduler (cron-style trigger)
│  ├─ deletions.rs       DeletedBy, deletions audit-table types
│  ├─ annotations.rs     <turn_annotations> JSON sidecar parsing
│  ├─ compression.rs     zstd encode + SHA-256 verify (compressAfterDays)
│  ├─ eval.rs            recall@K / MRR harness (ablation, manifest replay)
│  ├─ telemetry.rs       classify_turn, ClassifiedItem, TelemetryReport
│  ├─ services.rs        MemoryServices bundle (writer + observer + cfg)
│  ├─ trust_defaults.rs  MemorySource → default Trust mapping
│  ├─ kind.rs            MemoryKind enum (Decision/Convention/Invariant/…)
│  ├─ observer.rs        MemoryObserver, ManifestObserver
│  └─ reembed_migration.rs B1 cross-embedder /reembed migration
├─ swarm/                6-phase orchestration
│  ├─ models.rs          WorkUnit, AgentManifest, AgentStatus, SwarmResult
│  ├─ plan.rs            CompiledPlan, PlanNode, DependencyEdge, hash()
│  ├─ pipeline.rs        execute(): validate → execute → merge → verify →
│  │                     cleanup → consolidate
│  ├─ coordinator.rs     plan_coordinated: NLP task → CompiledPlan (LLM)
│  ├─ planner.rs         Static planner utilities
│  ├─ replanner.rs       ReplanDecision + Replanner (Phase 3 stub today)
│  ├─ calibration.rs     TierStats; per-run accuracy persisted to memory
│  ├─ context.rs         Repository context collection
│  ├─ context_bundle.rs  SwarmContextBundle (1 shared memory query +
│  │                     per-unit GraphSlice)
│  ├─ validation.rs      Pairwise scope overlap, topological sort
│  ├─ router.rs          TierRouter: (tier, privacy) → ResolvedBackend
│  ├─ privacy.rs         PrivacyScanner (glob privacy override)
│  ├─ execution_state.rs Checkpoint / resume
│  ├─ merge.rs           Claude-assisted merge conflict resolution
│  ├─ bus.rs             AgentBus (broadcast + targeted)
│  ├─ board.rs           SharedBoard (tagged agent findings)
│  ├─ ollama.rs          Legacy direct Ollama generator (kept for tests)
│  ├─ backend/           AgentBackend trait + impls
│  │  ├─ mod.rs          UnifiedStreamEvent, Capabilities, AgentBackend,
│  │  │                  BackendConfig (ClaudeCode / Codex / Cursor /
│  │  │                  Ollama / Custom)
│  │  ├─ shared.rs       backend_config_for_model, validate_model_spec,
│  │  │                  default_editor_system_prompt
│  │  ├─ executor.rs     complete_to_text, complete_to_write_gate
│  │  ├─ runner.rs       Per-unit runner orchestration
│  │  ├─ claude_code.rs  ClaudeCodeBackend
│  │  ├─ codex.rs        CodexBackend (exec)
│  │  ├─ cursor.rs       CursorBackend (NDJSON stream-json)
│  │  ├─ ollama.rs       OllamaStreamBackend
│  │  └─ mock.rs         MockBackend
│  └─ verify/            Structural | DiffReview | TestSuite | Combined
│     ├─ structural.rs   StructuralReport
│     ├─ diff_review.rs  DiffReviewReport (LLM diff critique)
│     ├─ test_runner.rs  TestReport (cargo test)
│     ├─ combined.rs     CombinedReport
│     └─ mod.rs          VerificationStrategy, BatchStrategy, VerificationStep
├─ iteration/            IterationEngine (retry, BestOfN, TDD)
│  ├─ mod.rs             IterationEngine; iteration policy
│  ├─ convergence.rs     Convergence detection between attempts
│  └─ test_generator.rs  TDD red-phase test synthesis
└─ validation_gate/      ValidationPipeline
   ├─ mod.rs             ValidationPipeline, ValidationGate trait
   ├─ tree_sitter_gate.rs Structural validity (parse errors)
   └─ cargo_gate.rs       `cargo check` / `cargo test`
```

---

## 2. Core Data Structures

**`FileScope` ([`types.rs`](src/types.rs))** — `owned_paths`, `read_only_paths`, `interface_contracts`. Glob-matched via [`path_pattern`](src/path_pattern.rs). Pairwise overlap uses `path_pattern::patterns_overlap`.

**`WorkUnit` ([`swarm/models.rs`](src/swarm/models.rs))** — id, description, scope, depends_on, coordinator instructions, model / tier / privacy, retries + escalation, memory routing, context expansion (`impact_scope`, `context_callers_of`, `context_tests_for`, `context_depth`).

**`CompiledPlan` ([`swarm/plan.rs`](src/swarm/plan.rs))** — `DiGraph<PlanNode, DependencyEdge>` + iteration / verification / loop config. Methods: `work_units_ordered`, `from_work_units`, `hash`.

**`UnifiedStreamEvent` ([`swarm/backend/mod.rs`](src/swarm/backend/mod.rs))** — `TextDelta | ThinkingDelta | ToolCallStart/Delta/End | FileBlock | Usage | Error | Done`.

**`BackendConfig` ([`swarm/backend/mod.rs`](src/swarm/backend/mod.rs))** — tagged enum: `ClaudeCode { model } | Codex { model } | Cursor { model } | Ollama { model, base_url } | Custom`. Materialized via `create_backend`.

**`Turn` / `TransportContext` ([`agent_session/mod.rs`](src/agent_session/mod.rs))** — `Turn` is a lossless lift of `PlannerSelections`; `TransportContext` carries `user_message`, `effort`, `auto_approve`. [`build_turn`](src/agent_session/mod.rs) is the single conversion (round-trip tested).

**`PlannerSelections` ([`context_planner/types.rs`](src/context_planner/types.rs))** — `MemorySelection`s, `GraphSelection`s, `FileAttachment`s, `ReplayPayload`, `ContinuityHandle`, `PlannerMetadata`, `ProviderProfile`. Produced by [`ContextPlanner::plan`](src/context_planner/mod.rs).

**`ContinuityHandle`** — `ClaudeSessionId(String) | CodexConversationId(String) | CursorThreadId(String) | None`. Surfaced via `AcpObserver::on_cursor_session_started` / equivalents; persisted on the `SessionLedger`.

**`MemoryScope` / `WriteScope` ([`memory/scope.rs`](src/memory/scope.rs))** — 5-level (`Global=0` → `Run=4`). `StoreKind` routes to one of three SQLite files via `MemoryStores`. `WriteScope` is always explicit.

**`SwarmContextBundle` ([`swarm/context_bundle.rs`](src/swarm/context_bundle.rs))** — built once per swarm run: `architectural_intent`, `shared_memory: Vec<MemoryCandidate>`, `per_unit_graph: HashMap<unit_id, GraphSlice>`. Cuts memory queries from N+1 to ≤2.

---

## 3. Swarm Pipeline (6 phases)

```
VALIDATE     pairwise scope overlap (path_pattern), Kahn topo-sort,
             dependency_tiers → Vec<Vec<WorkUnit>>; build SwarmContextBundle
EXECUTE      per tier: for each unit (Semaphore-bounded):
               git worktree (gaviero/{unit_id})
               McpConfigSynth → write per-worktree .mcp.json (Claude),
                                .codex/config.toml (Codex, if granted),
                                .cursor/mcp.json (Cursor)
               IterationEngine::run
                 attempts loop with escalation:
                   ContextPlanner::plan → PlannerSelections → Turn
                   AgentSession::send_turn → UnifiedStreamEvent stream
                   each FileBlock → write_gate.insert_proposal
                   (Cursor: tool calls drive snapshot+revert →
                    write_gate.insert_proposal carrying the agent's
                    intended content; backend Capabilities advertises
                    supports_file_blocks=false)
                   ValidationPipeline::run → corrective retry on fail
               Checkpoint ExecutionState
               (Replanner::evaluate on hard failure; today returns Continue)
MERGE        git merge --no-ff main; MergeResolver (Claude) on conflict
VERIFY       Structural | DiffReview | TestSuite | Combined; escalate on fail
CLEANUP      WorktreeManager::teardown_all, drop gaviero/* branches,
             remove per-worktree MCP configs
CONSOLIDATE  Consolidator (triage ≥ 0.4 → decay → cross-scope promotion);
             TierStats persisted to memory for adaptive tier calibration
```

---

## 4. Memory

### 4.1 Storage (multi-DB)

Three SQLite files per `MemoryStores`:
- `~/.config/gaviero/memory.db` — `Global`
- `<workspace>/.gaviero/memory.db` — `Workspace` + `Run`
- `<folder>/.gaviero/memory.db` — `Repo` + `Module` (per workspace folder)

A directly-opened single directory collapses workspace and folder to one file. Folder DBs are pre-registered but lazy-opened on first read/write. [`MemoryStores::open`](src/memory/stores.rs) runs the v10 split migration (refusable via `C1MigrationProposal` at the bootstrap layer).

### 4.2 Embedder

`Embedder` trait + [`model_manager::resolve_embedder_model`](src/memory/model_manager.rs). Default `gte-modernbert-base` (768 dim, mean-pool + L2 norm); legacy `nomic-embed-text-v1.5` selectable via `GAVIERO_EMBEDDER_MODEL` or `memory.embedder.model`. `e5-small-v2` and `null` available; `dual:<a>,<b>` runs an A/B comparison logged to `memory_embedder_ab`. The `api-embedders` Cargo feature reserves a hosted-API embedder surface but currently exposes a `NotImplemented` placeholder.

### 4.3 Writes (single-consumer)

```
caller → WriterHandle::send(WriterMessage)        (mpsc, bounded)
                              │
                              ▼
                writer task body (memory/writer.rs)
                              │
                              ├─ embed (no lock)
                              ├─ BRIEF LOCK   dedup probe (SHA-256 +
                              │               cosine ≥ 0.95) → reinforce /
                              │               skip-if-broader / insert
                              └─ optional oneshot ack (500ms)
```

`#![deny(clippy::await_holding_lock)]`. No callsite holds the SQLite Mutex during embed.

### 4.4 Retrieval

[`memory::retrieve_ranked`](src/memory/retrieval.rs) is the single funnel (chat injection, MCP `memory_search`, TUI memory panel, eval harness all go through it).

```
1. NO LOCK   embedder.embed(query)
2. MemoryStores::search_scoped (per RetrievalConfig.mode):
   a. merged   (B3 default): admit Global/Workspace/Repo/Module per ScopeMix,
               run vec + FTS in each, merge via RRF (vec 0.7, fts 0.3)
   b. cascade  (kill-switch): narrowest→widest per-level vec+FTS,
               EXIT if best_score > 0.70
3. NO LOCK   composite score = sim*0.5 + importance*0.2 + recency*0.15 + 0.15,
             scaled by scope/trust weights; B4 recency floor + decay-exempt
             types (Decision/Convention/Invariant/Preference)
4. NO LOCK   optional cross-encoder reranker: raw logit → sigmoid_calibrate →
             blend_rerank(w * cal + (1-w) * composite)
5. NO LOCK   dedup by content_hash; persist injection_manifests row (S4)
6. return Vec<ScoredMemory> + CandidatePoolEntry trace
```

### 4.5 Three-cadence consolidation

```
PER-TURN     extractor.rs (Tier S3): classify + emit candidate signals from
             the just-completed transcript via the consolidation LLM
PER-SESSION  session_consolidator.rs (B5): merge candidate briefs across the
             session, propose promotions, dedup, score
IDLE/WEEKLY  sleeptime.rs + sleeptime_scheduler.rs (B5): decay sweep
             (exp(-0.023 * days)), near-duplicate merge, cross-scope
             promotion (≥ 3 module hits → repo, ×1.2), trust re-scoring,
             history compression (zstd after compressAfterDays default 90),
             summary prune
```

### 4.6 Soft delete + audit

`/forget` writes a `deletions` audit row ([`deletions.rs`](src/memory/deletions.rs) + [`store/deletions_ops.rs`](src/memory/store/deletions_ops.rs)). History rows are immutable except via the C2.4 `user_redaction` redaction path (irreversible). Restore-by-id and restore-since-window replay through the dedup pipeline; `user_redaction` rows are skipped silently.

---

## 5. Backend Abstraction

[`AgentBackend`](src/swarm/backend/mod.rs) produces `Stream<Item = Result<UnifiedStreamEvent>>`. Implementations:

- [`ClaudeCodeBackend`](src/swarm/backend/claude_code.rs) — Claude via ACP subprocess.
- [`CodexBackend`](src/swarm/backend/codex.rs) — `codex exec` one-shot.
- [`CursorBackend`](src/swarm/backend/cursor.rs) — Cursor CLI `agent -p --output-format stream-json`. Capabilities advertise `supports_file_blocks=false`; native tool calls write to disk inside the worktree (swarm path) — review-flow snapshot+revert lives in the chat-side `CursorSession`.
- [`OllamaStreamBackend`](src/swarm/backend/ollama.rs) — Ollama HTTP SSE.
- [`MockBackend`](src/swarm/backend/mock.rs) — tests.

**Model spec ([`backend_config_for_model`](src/swarm/backend/shared.rs)):**

```
claude:<name>               → BackendConfig::ClaudeCode
codex:<name>                → BackendConfig::Codex          (exec)
cursor:<name>               → BackendConfig::Cursor         (default composer-2.5)
ollama:<name>               → BackendConfig::Ollama
local:<name>                → BackendConfig::Ollama         (alias)
```

`validate_model_spec` rejects bare names — provider prefix is required (supported: `claude`, `codex`, `cursor`, `ollama`, `local`). [`router::TierRouter`](src/swarm/router.rs) maps `(ModelTier, PrivacyLevel) → ResolvedBackend`. [`privacy::PrivacyScanner`](src/swarm/privacy.rs) glob-promotes a unit to `LocalOnly` (e.g. matches against `**/*.key`, `**/.env`).

---

## 6. Context Planner + Agent Session (V9)

```
PlannerInput ──► ContextPlanner::plan ──► PlannerSelections
                                              │
                                              ▼
                                    agent_session::build_turn
                                              │
                                              ▼   (lossless lift, round-trip-tested)
                                            Turn
                                              │
                                              ▼
                              AgentSession::send_turn(Turn)
              (Claude / CodexExec / CodexAppServer / Cursor / Ollama / Legacy)
                                              │
                                              ▼
                              Stream<UnifiedStreamEvent>
```

The planner is the single owner of memory queries, graph selection, replay, and continuity. `ContinuityMode` ∈ `Stateless | StatelessReplay | ProcessBound | NativeResume`. [`registry::create_session`](src/agent_session/registry.rs) routes by `ProviderProfile`:

| Provider | Mode | Session |
|---|---|---|
| `claude` | NativeResume | `ClaudeSession` |
| `cursor` | NativeResume | `CursorSession` (`agent --resume <thread-id>`) |
| `codex` | StatelessReplay | `CodexExecSession` |
| `codex-app` | ProcessBound | `CodexAppServerSession` (wrapped in `ObservedStreamSession`) |
| `ollama` / `local` | StatelessReplay | `OllamaSession` |

[`LegacyAgentSession`](src/agent_session/mod.rs) wraps [`AcpPipeline`](src/acp/client.rs) for byte-identical migration; per-provider impls replace it progressively.

**Two-layer graph context (first turn):** `<repo_topology>` is a cheap filesystem-only folder map ([`repo_map/topology.rs`](src/repo_map/topology.rs), `agent.topology.*` budget, default 600 tokens). `<repo_outline>` is the ranked PageRank file list (`agent.graphBudgetTokens`, default 12k). `/lite` keeps topology and drops outline, memory, and impact. Mid-turn relational context stays on MCP `blast_radius`.

[`chat_memory::perform_injection`](src/context_planner/chat_memory.rs) runs the per-turn retrieval inline; `chat_memory::enqueue_post_turn` schedules the S3 extractor + transcript writer through `WriterHandle`.

---

## 7. MCP (in-process server)

Listens on `<workspace>/.gaviero/mcp.sock`. Subprocess agents reach it through the [`gaviero-mcp-shim`](../gaviero-mcp-shim) binary (stdio↔socket bridge), declared as their MCP server in synthesized per-worktree configs.

**Three read-only tools:**

| Tool | Backed by | Output |
|---|---|---|
| `memory_search(query, scope_hint?, limit?)` | `retrieve_ranked` (same path as chat injection) | `Vec<MemorySearchResult>` |
| `blast_radius(paths, depth?, mode?)` | `RepoMap` typed graph + mode-weighted PageRank + C3 specificity + C4 edge-weight overrides | `Vec<BlastRadiusRelation>` |
| `node_doc(path)` | Tier D1 schema stub | `NodeDoc` (signatures today; `purpose` empty pending D1) |

**Read-only invariant.** [`GavieroMcpServer`](src/mcp/server.rs) carries `Arc<MemoryStores>` + a graph cache, but **no `WriterHandle`** — `memory_store` / `_update` / `_delete` cannot exist. Writes flow through the S2 writer task (transcripts + annotations) only.

[`mcp::config_synth`](src/mcp/config_synth.rs) writes per-worktree configs:

- Claude Code → `<worktree>/.mcp.json`.
- Codex → `<worktree>/.codex/config.toml` (gated on `TrustConsent::Granted` — one-time prompt).
- Cursor → `<worktree>/.cursor/mcp.json` (same `{"mcpServers":{...}}` schema as Claude; aliased via [`cursor_mcp_config_json`](src/mcp/config_synth.rs)).

[`mcp::external_memory`](src/mcp/external_memory.rs) detects competing memory MCP servers in agent config and disables them with consent. [`McpToolCallObserver`](src/mcp/observer.rs) logs every call (`McpCallLogEntry`) for the TUI audit panel.

---

## 8. Write Gate

Modes: `Interactive` (queue → TUI review), `AutoAccept` (validate + write), `Deferred` (batch), `RejectAll` (drop silently).

```
write(path, content)
  ├─ BRIEF LOCK   is_scope_allowed(agent_id, path)  [path_pattern]
  ├─ NO LOCK      compute_hunks → enrich_hunks (StructuralHunk)
  ├─ BRIEF LOCK   insert_proposal (mode-specific)
  └─ NO LOCK      fs::write if finalized
```

Observers fire `on_proposal_created / updated / finalized`. UI can accept/reject per hunk or per AST node (`enclosing_node`).

---

## 9. ACP (Claude subprocess, legacy)

[`AcpSessionFactory`](src/acp/factory.rs) spawns `one_shot` or `persistent` sessions. [`AcpSession`](src/acp/session.rs) handles NDJSON over stdin/stdout; large prompts spill to a tempfile (`ARGV_THRESHOLD` in `session.rs`). Events: `SystemInit`, `ContentDelta`, `ToolUseStart`, `AssistantMessage`, `ResultEvent` ([`protocol.rs`](src/acp/protocol.rs)). [`AcpPipeline`](src/acp/client.rs) enriches prompts and routes detected file blocks through the write gate. New code paths go through `agent_session::claude` instead, but the legacy path remains until M10 parity.

---

## 10. Concurrency

| Component | Primitive | Rule |
|---|---|---|
| [`WriteGatePipeline`](src/write_gate.rs) | `tokio::sync::Mutex` | No lock across diff, parse, I/O |
| `MemoryStore` (×N) | `tokio::sync::Mutex<rusqlite::Connection>` per DB | Embed outside lock; brief DB ops; writes via writer task |
| MCP graph cache | `tokio::sync::Mutex<GraphStore>` | Lazy init, reused; serializes `blast_radius` calls |
| `ExecutionState` | `Mutex<Vec<NodeStatus>>` | Checkpoint after each node |
| Writer task input | `mpsc::UnboundedSender<WriterMessage>` | Single consumer; optional oneshot ack |
| [`AgentBus`](src/swarm/bus.rs) | `broadcast::channel` | Lock-free |
| Parallel agents | `Semaphore` | Bounded per tier |

**Never hold a Mutex across `await`, tree-sitter parse, or `fs` I/O.** Enforced via `#![deny(clippy::await_holding_lock)]` in [`memory/writer.rs`](src/memory/writer.rs).

---

## 11. Error Handling

| Error | Recoverable | Handling |
|---|---|---|
| parse/compile (DSL) | compile-time | miette diagnostic with span |
| scope violation | no | reject proposal, observer, log |
| agent failure | yes | `AgentStatus::Failed` → escalate or replan |
| validation gate | yes | corrective feedback → retry |
| merge conflict | yes | Claude resolution or user choice |
| memory init | non-fatal | `Option<Arc<MemoryStores>>`, continue |
| C1 migration pending | no | refuse open until consent (proposal surfaced to bootstrap) |
| MCP server bind failure | non-fatal | log; subprocess agents fall back |
| Cursor argv-limit overflow | no | reject prompt with explicit error (96 KB ceiling) |
| consolidation / cleanup | non-fatal | log, continue |

---

## 12. Hard Constraints

1. All agent writes through [`WriteGatePipeline`](src/write_gate.rs).
2. `git2` only — never shell out to `git`.
3. Tree-sitter for syntax, highlight, indent (16-language registry).
4. `MemoryStore` behind `tokio::sync::Mutex`; embedding outside lock; writes via the single writer task only.
5. MCP is read-only by construction — no write tools.
6. No UI / DSL types in core.
7. [`AgentBackend`](src/swarm/backend/mod.rs) + `UnifiedStreamEvent` and [`AgentSession`](src/agent_session/mod.rs) + `Turn` are provider-agnostic; selection via `TierRouter` + `PrivacyScanner`.
8. Explicit `WriteScope` — never inferred.
9. Swarm branches `gaviero/{work_unit_id}`; worktrees `.gaviero/worktrees/{id}/`, cleanup via `Drop`. MCP configs are per-worktree.
10. Model specs require provider prefix; `validate_model_spec` is authoritative.

---

## 13. Public API (22 pub mods)

```rust
// crates/gaviero-core/src/lib.rs
pub mod acp;              pub mod agent_session;   pub mod context_planner;
pub mod mcp;              pub mod memory;          pub mod swarm;
pub mod repo_map;         pub mod write_gate;      pub mod validation_gate;
pub mod iteration;        pub mod workspace;       pub mod git;
pub mod session_state;    pub mod path_pattern;    pub mod scope_enforcer;
pub mod observer;         pub mod types;           pub mod tree_sitter;
pub mod diff_engine;      pub mod query_loader;    pub mod indent;
pub mod terminal;

pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};
```

Downstream crates must **only** use the tree-sitter re-exports — never `use tree_sitter::*` directly. `repo_map::topology` (`TopologyConfig`, `build_folder_topology`) is publicly re-exported at the module root for `<repo_topology>` callers.

---

See [CLAUDE.md](CLAUDE.md) for conventions, build, and rules.
