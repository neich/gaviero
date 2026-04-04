# gaviero-core — Architecture

`gaviero-core` is the shared execution engine for Gaviero. It owns all domain logic: workspace configuration, agent I/O (ACP), write-gated proposal review, multi-agent swarm orchestration, semantic memory, git integration, scope enforcement, syntax analysis, and context-budget planning. The `gaviero-tui` and `gaviero-cli` binaries are thin shells that implement observer traits and call into this library.

---

## Module map

| Module | Purpose |
|---|---|
| `types` | Canonical shared types: `WorkUnit`, `WriteProposal`, `FileScope`, `ModelTier`, `PrivacyLevel`, `StructuralHunk`, `normalize_path()` |
| `workspace` | Settings cascade (user → workspace → folder); resolves model names, namespaces, layout |
| `session_state` | Persisted UI state (open tabs, pane layout, file-tree expansion, terminal session) |
| `observer` | Three observer traits (`WriteGateObserver`, `AcpObserver`, `SwarmObserver`) — the coupling surface between core and frontends |
| `diff_engine` | Line-based diff via `similar`; produces `StructuralHunk` vectors with context lines |
| `tree_sitter` | Language detection; enriches diff hunks with enclosing AST nodes (function/class names) |
| `indent/` | Auto-indent engine: tree-sitter query path first, bracket-count fallback |
| `query_loader` | Loads tree-sitter highlight and indent query files from disk |
| `acp/` | Agent Communication Protocol: spawn `claude` subprocess, stream NDJSON events, route proposals through WriteGate |
| `write_gate` | Proposal lifecycle: stage → hunk accept/reject → assemble → write to disk; scope enforcement |
| `scope_enforcer` | File-access control enforcement: owned-path checks and hard block-list for credentials/secrets |
| `validation_gate/` | Inline validation pipeline: tree-sitter (fast) + cargo check (slow); produces corrective prompts for agent retry |
| `repo_map/` | Personalized PageRank over tree-sitter symbol graph; produces `ContextPlan` for prompt budget allocation |
| `terminal/` | PTY management, vt100 emulation, shell integration, terminal history |
| `swarm/` | Multi-agent orchestration: coordinator, pipeline, tier routing, backends, verification, merge |
| `memory/` | Semantic memory: SQLite + sqlite-vec, ONNX embeddings, namespace isolation, privacy filtering, consolidation |
| `git` | Git repo operations via `git2`; worktree lifecycle via CLI |

---

## Core type hierarchy

```
WorkUnit
├─ id: String
├─ description: String
├─ scope: FileScope { owned_paths, read_only_paths, interface_contracts }
├─ depends_on: Vec<String>           ← dependency graph edges
├─ tier: ModelTier                   ← Coordinator | Reasoning | Execution | Mechanical
├─ privacy: PrivacyLevel             ← Public | LocalOnly
├─ coordinator_instructions: String  ← decomposed task text
├─ model: Option<String>             ← override; None → TierRouter picks
├─ read_namespaces / write_namespace ← memory routing
├─ max_retries: u8
├─ escalation_tier: Option<ModelTier>
├─ staleness_sources: Vec<String>    ← file paths; hash-checked before each run
└─ memory_importance: Option<f32>    ← 0.0–1.0; default 0.5

WriteProposal
├─ id: u64
├─ source: String                    ← agent ID
├─ file_path: PathBuf
├─ original_content: String
├─ proposed_content: String
├─ structural_hunks: Vec<StructuralHunk>
│   └─ DiffHunk + enclosing_node_name + description + status: HunkStatus
└─ status: ProposalStatus            ← Pending | PartiallyAccepted | Accepted | Rejected

SwarmResult
├─ manifests: Vec<AgentManifest>
│   └─ work_unit_id, status: AgentStatus, modified_files, branch, summary, cost_usd
├─ merge_results: Vec<MergeResult>
│   └─ success, conflicts: Vec<MergeConflict>
├─ success: bool
└─ pre_swarm_sha: String             ← HEAD before run (for undo-swarm)
```

---

## Observer pattern

Three traits in `observer.rs` decouple core execution from frontend rendering. Frontends implement the traits and pass boxed implementations into core functions. Observers must never block — they only fire-and-forget into an event channel.

### `WriteGateObserver`

| Method | When fired |
|---|---|
| `on_proposal_created(proposal)` | New diff ready; proposal is boxed and sent to frontend |
| `on_proposal_updated(id)` | A hunk status changed (accept/reject) |
| `on_proposal_finalized(path)` | Final content written to disk |

### `AcpObserver`

| Method | When fired |
|---|---|
| `on_stream_chunk(conv_id, text)` | Response text delta from Claude subprocess |
| `on_tool_call_started(conv_id, tool)` | Agent issued Read / Grep / Write / Edit |
| `on_streaming_status(conv_id, status)` | Spinner text (e.g. "Building plan…") |
| `on_message_complete(conv_id, role, content)` | Agent turn complete |
| `on_proposal_deferred(conv_id, path, +, -)` | File diff deferred for batch review |
| `on_validation_result(conv_id, gate, result)` | Gate pass/fail notification |
| `on_validation_retry(conv_id, attempt)` | Retry after validation failure |

### `SwarmObserver`

| Method | When fired |
|---|---|
| `on_phase_changed(phase)` | "validating" / "running" / "merging" |
| `on_agent_state_changed(id, status, detail)` | Pending → Running → Completed / Failed |
| `on_tier_started(current, total)` | New dependency tier begins |
| `on_merge_conflict(branch, files)` | Merge conflict detected |
| `on_coordination_started(prompt)` | Coordinator Opus call begins |
| `on_coordination_complete(unit_count, summary)` | DSL plan ready |
| `on_tier_dispatch(unit_id, tier, backend)` | Agent assigned to backend |
| `on_cost_update(estimate)` | Token / cost estimate updated |
| `on_completed(result)` | `SwarmResult` ready |

---

## ACP subsystem (`acp/`)

ACP (Agent Communication Protocol) is a subprocess-based agent interface. It spawns the `claude` CLI and streams NDJSON events over stdout.

### Components

**`AcpSession` (`session.rs`)** — low-level subprocess wrapper:
- Spawns: `claude --print --output-format stream-json --model <model> --add-dir <cwd>`
- Writes prompt via stdin, then closes stdin
- `next_event() → Result<Option<StreamEvent>>` reads NDJSON lines one by one

**`StreamEvent` (`protocol.rs`)** — parsed NDJSON events:

| Variant | Meaning |
|---|---|
| `SystemInit { session_id, model }` | Subprocess ready |
| `ContentDelta(text)` | Streaming response chunk |
| `ThinkingDelta(text)` | Extended thinking chunk |
| `ToolUseStart { tool_name, tool_use_id }` | Agent is about to call a tool |
| `ToolInputDelta(json)` | Partial tool input (streaming) |
| `AssistantMessage { text, tool_uses }` | Complete assistant turn |
| `ResultEvent { result_text, cost_usd, duration_ms, .. }` | Final result + metrics |
| `Unknown(json)` | Unrecognised event — logged and skipped |

**`AcpPipeline` (`client.rs`)** — high-level agent runner:
```
send_prompt(prompt, history, attachments, observer, write_gate)
  → enrich prompt (conversation history, referenced file contents)
  → spawn AcpSession (allowed tools: Read, Glob, Grep, Write, Edit, MultiEdit)
  → stream events → observer callbacks
  → parse <file path="...">content</file> blocks → WriteGatePipeline::propose()
```

---

## WriteGate pipeline (`write_gate.rs`)

All agent-proposed file writes flow through `WriteGatePipeline`. Nothing reaches disk until the user (or `AutoAccept` mode) finalises the proposal.

### WriteMode

| Mode | Behaviour |
|---|---|
| `Interactive` | Proposals queued; observer notified; user accepts/rejects hunks in UI |
| `AutoAccept` | All hunks auto-accepted; content written immediately |
| `RejectAll` | All proposals silently discarded |
| `Deferred` | Proposals accumulated; no observer; released as batch on `take_pending_proposals()` |

### Proposal lifecycle

```
Agent emits <file> block
  → AcpPipeline calls WriteGatePipeline::build_proposal()
      ├─ diff_engine::compute_hunks(original, proposed)
      ├─ tree_sitter::enrich_hunks()  ← adds enclosing function/class
      └─ returns WriteProposal (all hunks Pending)
  → propose(proposal)
      ├─ scope check: is_scope_allowed(agent_id, path)  ← fail-closed if no scope
      └─ route by WriteMode

User interacts (Interactive / batch):
  accept_hunk(id, index)   → HunkStatus::Accepted
  reject_hunk(id, index)   → HunkStatus::Rejected
  accept_all(id) / reject_all(id)
  accept_node(id, node)    → accept all hunks in named enclosing node

Finalise:
  assemble_final_content(id)
    → iterate original lines
    → splice in accepted hunks only
    → rejected / pending hunks keep original text
  write to disk → observer.on_proposal_finalized(path)
```

---

## Scope enforcement (`scope_enforcer.rs`)

`ScopeEnforcer` is a separate gate from `WriteGatePipeline.is_scope_allowed()`. It enforces per-agent `FileScope` declarations and applies a hard block-list regardless of agent scope.

### Hard block-list (always denied)

`.env*`, `id_*`, `.ssh/`, `.aws/`, `.netrc`, `secrets.*`, and similar credential paths.

### Methods

| Method | Behaviour |
|---|---|
| `check_write(agent_id, path)` | Blocks: sensitive block-list paths; paths outside `owned_paths`; unregistered agents (fail-closed) |
| `check_read(agent_id, path)` | Allows sensitive reads only if path is explicitly in `read_only_paths`; blocks otherwise |

`ScopeViolation` carries the path and reason string for error reporting.

---

## Validation gate pipeline (`validation_gate/`)

Inline validation runs after each agent write turn. Gates are chained — first failure short-circuits remaining gates.

### `ValidationGate` trait

```rust
trait ValidationGate {
    fn name(&self) -> &str;
    fn is_fast(&self) -> bool;    // fast gates run after every write; slow gates run at checkpoints
    fn validate(&self, path: &Path, content: &str) -> ValidationResult;
}
```

### Built-in gates

| Gate | Speed | What it checks |
|---|---|---|
| `TreeSitterGate` | Fast | Syntax validity via tree-sitter parse; runs after every write turn |
| `CargoCheckGate` | Slow | Type-checks the full crate via `cargo check`; runs at tier checkpoints |

### `corrective_prompt(gate_name, result) → String`

Formats a validation failure into an agent-readable correction instruction. The runner appends this to the next prompt turn, enabling the agent to self-correct without external intervention.

### `ValidationResult`

`Pass` | `Fail { message, suggestion }` | `Skip` (gate not applicable to file type)

---

## Repo map (`repo_map/`)

Builds a file-relevance ranking from the workspace to allocate the agent's context budget efficiently.

### Algorithm

1. **Builder** (`builder.rs`): scans workspace files, extracts top-level symbols via tree-sitter; produces `Vec<FileNode { path, token_estimate, symbols }>`
2. **PageRank** (`page_rank.rs`): personalized PageRank seeded on agent-owned files; ranks all files by relevance
3. **ContextPlan**: fills token budget — owned files as `full_content`, high-rank files as `signatures`, remainder as `list`

### `ContextPlan` output modes

| Mode | Content injected into prompt |
|---|---|
| `full_content` | Complete file text (agent's owned files) |
| `signatures` | Top-level symbol names and line numbers only |
| `list` | File path only |

---

## Memory system (`memory/`)

Persistent semantic store backed by SQLite + `sqlite-vec` vector extension.

### Storage schema (v3)

| Table | Contents |
|---|---|
| `memories` | `(id, namespace, key, content, embedding BLOB, privacy, importance REAL, access_count, created_at, updated_at)` |
| `vec0_memories_embeddings` | Virtual table over `memories.embedding`; provides cosine distance search |
| `episodes` | Episodic memory events (session summaries) |
| `code_graph` | Codebase structure cache (extracted symbols, call graph) |

### Embedding model

- **ONNX Runtime** (`ort` crate) + **nomic-embed-text-v1.5** (768-dimensional)
- Downloaded to `~/.cache/gaviero/` on first use via `model_manager.rs`
- `Embedder` trait: `embed(text: &str) → Vec<f32>`
- Retrieval score: `recency × importance × cosine_similarity` (Stanford Generative Agents formula)

### Key operations

```rust
store.store(namespace, key, content) → i64
store.store_with_options(..., privacy, importance, source_file, source_hash)
store.search(namespaces, query, limit) → Vec<SearchResult { entry, score }>
store.search_context_filtered(namespaces, query, limit, PrivacyFilter::ExcludeLocalOnly)
    → String  // Markdown-formatted context block for agent prompts
store.get(namespace, key) → Option<MemoryEntry>
```

### Privacy filtering

Entries written by `LocalOnly` agents are tagged. `PrivacyFilter::ExcludeLocalOnly` strips them from search results before sending context to cloud-hosted models.

### Staleness detection

Agents declare `staleness_sources: Vec<String>`. Before each run, `pipeline::invalidate_stale_sources()` hashes each listed file. If any hash differs from the stored snapshot, cached memory entries for that agent are invalidated and the agent runs fresh.

### Consolidation (`consolidation.rs`)

`Consolidator` deduplicates near-duplicate entries on write:

| Similarity | Action |
|---|---|
| > 0.85 | Reinforce existing (increment `access_count`) |
| 0.70 – 0.85 | Flag for LLM merge (background `sweep()`) |
| < 0.70 | Insert as new entry |

---

## Swarm subsystem (`swarm/`)

### Submodule map

| Submodule | Purpose |
|---|---|
| `pipeline` | Top-level `execute()` and `plan_coordinated()` entry points |
| `coordinator` | Opus call → `plan_as_dsl()` emits `.gaviero` DSL text (primary); `plan()` JSON TaskDAG path (deprecated) |
| `plan` | `CompiledPlan`: immutable petgraph `DiGraph<PlanNode, DependencyEdge>`; `work_units_ordered()` topological sort |
| `models` | `WorkUnit`, `AgentManifest`, `SwarmResult`, `AgentStatus`, `MergeResult` |
| `router` | `TierRouter`: `(tier, privacy, ollama_available)` → `(backend, model)` |
| `backend/` | `AgentBackend` trait + `ClaudeCode`, `Ollama`, `Mock` implementations; `runner.rs` |
| `verify/` | `VerificationStrategy` enum + `StructuralOnly`, `DiffReview`, `TestSuite`, `Combined` |
| `merge` | Git merge per-agent branch; Claude-assisted conflict resolution |
| `validation` | Scope overlap checks (`validate_scopes`), dependency cycle detection (`dependency_tiers`) |
| `board` | `SharedBoard`: inter-agent discovery sharing via `[discovery: <tag>]` annotations |
| `execution_state` | `ExecutionState`: per-node `NodeStatus` tracking; checkpoint/resume support |
| `replanner` | `Replanner`: evaluates mid-run failures; decides continue / retry / revise / abort |
| `calibration` | Token / cost estimation; per-tier accuracy stats stored to memory |
| `privacy` | `PrivacyScanner`: glob-based path overrides for routing; safety net over coordinator decisions |
| `bus` | `AgentBus`: inter-agent message passing (future use) |
| `context` | Enrich agent prompt with workspace file list + memory context |

### Coordinator (`coordinator.rs`)

**Primary path: `plan_as_dsl()`**

```
plan_as_dsl(prompt, config, memory, observer)
  → Opus call with workspace file list + privacy-filtered memory context
  → streams text → observer.on_stream_chunk()
  → returns .gaviero DSL text (String)
  → caller writes to tmp/, user reviews, user calls /run → compile()
```

The coordinator generates a `.gaviero` file. The user reviews scopes and dependencies before any agent executes. This makes phantom file references visible and fixable before dispatch.

**Deprecated path: `plan()` JSON TaskDAG** — still compiles but is not exercised by current CLI/TUI code.

### Execution flow (`pipeline.rs`)

```
plan_coordinated(prompt, config, memory) → .gaviero DSL text  ← preferred entry
  → Coordinator::plan_as_dsl()
  → write to tmp/gaviero_plan_<ts>.gaviero
  → return path; user reviews; user calls /run

execute(work_units, config, memory, observer)
  1. validate_scopes(units)            ← error on owned-path overlaps
  2. dependency_tiers(units)           ← topological sort → Vec<Vec<String>>
  3. for each tier (sequential):
       for each unit in tier (parallel, semaphore-limited):
         a. provision git worktree      ← optional; falls back to shared workspace
         b. invalidate_stale_sources()  ← hash check staleness_sources
         c. enrich_context()            ← repo_map + memory search
         d. TierRouter::resolve(unit)   ← pick backend + model
         e. runner::run(unit, backend)  ← spawn AcpSession → collect proposals
              ├─ ScopeEnforcer checks each write
              ├─ ValidationGate runs after each write turn
              ├─ corrective_prompt() injected on failure → retry
              └─ SharedBoard::post_discoveries() after each turn
         f. commit agent branch
         g. ExecutionState::record_result()  ← checkpoint to disk
       merge all tier branches → collect MergeResult
  4. return SwarmResult { manifests, merge_results, success, pre_swarm_sha }
```

**Resume support**: `ExecutionState` is serialized to `.gaviero/state/{plan_hash}.json` after each node completes. With `--resume`, `execute()` loads this file and skips nodes whose status is already `Completed`.

### Shared board (`swarm/board.rs`)

Inter-agent discovery sharing within a swarm run.

- **`SharedBoard`**: in-memory `RwLock`-protected list of `SharedEntry { from_agent, content, tags }`
- **Discovery syntax** parsed from agent output: `[discovery: <tag>] <content>`
  - `<tag>` is a path-like label (e.g. `src/auth.rs`, `performance`) used for relevance filtering
- **`format_for_prompt(tags)`**: filters entries by tag/path overlap with the requesting agent's scope; formats as a section for prompt injection
- Enables downstream agents to benefit from upstream findings without explicit file dependencies

### Execution state (`swarm/execution_state.rs`)

```
NodeStatus: Pending | Blocked | Ready | Running | Completed | SoftFailure | HardFailure | Skipped

ExecutionState
├─ plan_hash: String               ← deterministic hash of CompiledPlan
├─ nodes: HashMap<id, NodeState>
│   └─ status, attempt_count, manifest, validation_issues, cost_usd, started_at, finished_at
└─ checkpoint path: .gaviero/state/{plan_hash}.json

Methods:
  set_status(id, status)
  record_result(id, manifest)
  all_terminal() → bool
  load(path) → Result<ExecutionState>   ← for --resume
```

### Privacy scanner (`swarm/privacy.rs`)

`PrivacyScanner` evaluates `PrivacyLevel` from glob patterns configured in workspace settings.

- `classify(work_unit)` → `PrivacyLevel`: matches unit's owned/read-only paths against sensitive-file glob patterns
- `apply_overrides(units)`: mutates units in-place, upgrading to `LocalOnly` when any path matches
- Acts as a safety net: coordinator routing decisions are never purely trusted; pattern-based overrides take precedence

### TierRouter decision table (`swarm/router.rs`)

| Tier | Privacy | Ollama | Result |
|---|---|---|---|
| any | LocalOnly | available | Ollama (mechanical config) |
| any | LocalOnly | unavailable | Blocked |
| Coordinator | Public | — | Claude `opus` |
| Reasoning | Public | — | Claude `reasoning_model` (config) |
| Execution | Public | — | Claude `execution_model` (config) |
| Mechanical | Public | available | Ollama |
| Mechanical | Public | unavailable | Blocked |
| any | — | model override set | Override model (privacy-checked) |

---

## Integration: what core exports

`lib.rs` re-exports the following as the public API surface for `gaviero-tui` and `gaviero-cli`:

```rust
pub mod workspace       → Workspace { load(), single_folder(), resolve_setting() }
pub mod types           → WorkUnit, WriteProposal, FileScope, ModelTier, PrivacyLevel, …
pub mod observer        → WriteGateObserver, AcpObserver, SwarmObserver
pub mod write_gate      → WriteGatePipeline, WriteMode
pub mod scope_enforcer  → ScopeEnforcer, ScopeViolation
pub mod session_state   → SessionState
pub mod diff_engine     → compute_hunks()
pub mod tree_sitter     → language_for_extension(), enrich_hunks()
pub mod git             → GitRepo, WorktreeManager, current_head_sha()
pub mod memory          → MemoryStore, init() [async init helper]
pub mod terminal        → TerminalManager, TerminalInstance, TerminalEvent
pub mod acp             → AcpPipeline, AcpSession, AgentOptions
pub mod swarm::pipeline → execute(), plan_coordinated()
pub mod swarm::models   → WorkUnit, AgentManifest, SwarmResult, SwarmConfig
pub mod swarm::plan     → CompiledPlan, PlanNode, DependencyEdge
pub mod swarm::coordinator → CoordinatorConfig
pub mod swarm::router   → TierConfig
```

### Typical consumer setup sequence

```rust
// 1. Workspace + settings
let ws = Workspace::single_folder(".")?;
let model = ws.resolve_setting("agent.model", Some(root))?;

// 2. Memory (optional; degrades gracefully if ONNX unavailable)
let memory = memory::init(None).await.ok().map(Arc::new);

// 3. Observer (implement traits, forward to UI channel)
struct MyObserver { tx: mpsc::UnboundedSender<MyEvent> }
impl SwarmObserver for MyObserver { … }

// 4. Execute
let result = swarm::pipeline::execute(
    work_units,
    &SwarmConfig { workspace_root, model, use_worktrees: true, … },
    memory,
    &observer,
    |unit_id| Box::new(my_acp_observer(unit_id)),
).await?;
```

---

## Key dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime; `spawn`, `Semaphore`, `Mutex` |
| `git2` | Git repo operations (status, branch, commit) |
| `similar` | Text diff (grouped hunks with context lines) |
| `tree-sitter` + 16 grammars | Syntax parsing for hunk enrichment, validation, auto-indent, repo_map |
| `petgraph` | Dependency graph (`CompiledPlan` DiGraph) and cycle detection |
| `rusqlite` + `sqlite-vec` | Memory store and vector search |
| `ort` (ONNX Runtime) | Embedding inference (nomic-embed-text-v1.5) |
| `tokenizers` | Tokenisation for ONNX model |
| `portable-pty` | PTY spawning for terminal emulation |
| `vt100` | VT100 screen emulation |
| `serde` / `serde_json` | Serialisation for NDJSON, config, session state |
| `reqwest` | HTTP client (Ollama backend) |
| `anyhow` | Error propagation |
| `tracing` | Structured logging |

---

## Design decisions

1. **Core as pure library.** All domain logic lives in `gaviero-core`; binaries hold no logic. Enables testing core behaviour without a UI.

2. **Observer-driven decoupling.** Three observer traits with default no-op implementations allow frontends to subscribe only to the events they care about, without polling or shared mutable state.

3. **Fail-closed write gate.** Agents with no registered scope are denied all writes (`None → false`). Prevents unintended file mutations from agents that bypass scope registration.

4. **Fail-closed scope enforcer.** Hard block-list denies writes to credential paths regardless of scope declarations. Agents can never be tricked into overwriting `.env`, SSH keys, or AWS credentials.

5. **Deferred write mode.** During streaming, proposals accumulate in `Deferred` mode. The full batch is released for review only after the agent turn completes, preventing mid-stream UI jank.

6. **DSL-first coordination.** `plan_coordinated()` produces a `.gaviero` text file rather than executing agents immediately. The user reviews the plan before dispatch, making phantom file references visible and fixable.

7. **Worktree isolation.** Each agent runs in a dedicated git worktree when `use_worktrees = true`. Isolation prevents cross-agent file conflicts during parallel execution.

8. **Privacy-layered memory.** `LocalOnly` entries are never included in context sent to cloud models. Enforced at retrieval time, not at storage time.

9. **Staleness-driven cache invalidation.** Source file hashes are stored alongside memory entries. Hash changes trigger re-execution rather than serving stale context.

10. **Inline validation with corrective prompts.** Validation gates run after each agent write turn. Failures produce structured corrective prompts injected into the agent's next turn, enabling self-correction without external intervention.

11. **SharedBoard for zero-dependency discovery sharing.** Agents post tagged discoveries during their run. Downstream agents in the same tier or later tiers receive filtered findings in their prompts — without requiring explicit file dependencies in the DAG.

12. **ExecutionState checkpointing.** Per-node state is written to disk after each completion. `--resume` loads the checkpoint and skips completed nodes, enabling partial recovery from mid-run failures.

13. **16+ language support.** Tree-sitter grammars cover Rust, Python, TypeScript, JavaScript, Java, C, C++, Go, Ruby, Kotlin, Swift, Scala, Haskell, Lua, TOML, and more. Language is detected from file extension.
