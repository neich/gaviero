# gaviero-core — Architecture

Shared execution layer. All runtime logic: swarm, memory, MCP, ACP / agent sessions (incl. DeepSeek `tool_agent`), write gate, validation, git, terminal, repo-map, skills, context planning. **No UI or DSL dependencies.**

Conventions and rules: [CLAUDE.md](CLAUDE.md). Workspace topology: [../../ARCHITECTURE.md](../../ARCHITECTURE.md).

---

## Topology

```
gaviero-core (lib, 28 pub mods)
 ├── swarm/ + agent_session/     orchestration + provider transport
 ├── memory/ + mcp/              scoped store + read-only MCP server
 ├── write_gate/ + scope_*       single write path for all agents
 ├── repo_map/ + skills/         graph / topology / skill catalog
 └── acp/ git/ terminal/ …      supporting subsystems
         │
         ├── re-exports tree-sitter 0.25 (+ tree-sitter-gaviero grammar)
         └── McpEndpoint (.gaviero/mcp.sock | \\.\pipe\gaviero-<hash>)
                   ▲
                   │ gaviero-mcp-shim (stdio bridge; not a crate dep)
         subprocess agents (claude / codex / cursor / dsh-acp)
         DeepSeek `deepseek:`: in-process tool_agent — no shim
         HTTP: 127.0.0.1:<derived-port>/mcp (bearer token)
```

Depends on: tokio, tree-sitter 0.25 (+ grammars), git2, rusqlite + sqlite-vec, ort + tokenizers, petgraph, portable-pty, rmcp, … — see [CLAUDE.md](CLAUDE.md) Dependencies. Downstream crates must **not** depend on `tree-sitter` directly; use re-exports from [`src/lib.rs`](src/lib.rs).

---

## Modules

**28 pub mods** from [`src/lib.rs`](src/lib.rs):

```
gaviero-core/src/
├─ lib.rs                 Re-exports tree-sitter types + 28 pub mods
├─ types.rs               FileScope, WriteProposal, ModelTier, PrivacyLevel, …
├─ workspace.rs           Workspace::single_folder / load, settings cascade
├─ session_state.rs       SessionState, TabState, StoredConversation, index
├─ tree_sitter.rs         LANGUAGE_REGISTRY (16 langs), enrich_hunks
├─ diff_engine.rs         compute_hunks
├─ write_gate.rs          WriteGatePipeline, WriteMode, proposal lifecycle
├─ turn_capture/          Git-free per-turn change capture (TUI chat review)
│  ├─ mod.rs              TurnCapture begin/end/resolve/gc, pending persistence
│  ├─ walk.rs / manifest.rs / store.rs   stat walk, baseline, SHA-256 blobs
│  ├─ changeset.rs / ledger.rs / revert.rs  change set, host writes, file/hunk revert
├─ observer.rs            WriteGateObserver, AcpObserver (+ on_tool_call_completed), SwarmObserver
├─ history/               Per-turn NDJSON log: record / writer (HistoryRecorder) / reader / tokens
├─ scope_enforcer.rs      FileScope checks → path_pattern
├─ path_pattern.rs        Glob matcher + patterns_overlap
├─ git.rs                 GitRepo, WorktreeManager, GitCoordinator (git2)
├─ git_conflict.rs        ConflictRegion / marker parsing (TUI F8/F9)
├─ query_loader.rs        Tree-sitter .scm discovery
├─ skills/                Frontmatter, catalog, planner ResolvedSkill seam
│  ├─ mod.rs / frontmatter.rs / catalog.rs / template.rs
├─ util/                  Shared helpers
│  ├─ fs.rs               Filesystem helpers
│  ├─ ndjson.rs           Size-rotated NDJSON appender (MCP telemetry + history)
│  └─ spawn.rs            Process spawn (+ Windows Job Objects)
├─ indent/                compute_indent (tree-sitter / hybrid / bracket)
├─ terminal/              PTY (portable-pty) + OSC 133
├─ repo_map/              Code graph + ranking
│  ├─ builder.rs / edges.rs / graph_builder.rs / page_rank.rs / store.rs
│  ├─ resolve.rs          Qualified, import-aware reference → edge resolution
│  ├─ resolve/receiver.rs Receiver typing: type facts → method targets
│  ├─ rust_walk.rs        Rust reference walk + type facts + receiver expressions
│  ├─ types.rs            Written types (TypeRef), receiver expressions, std model
│  ├─ extraction.rs       Shared extraction policy + per-path outcome
│  ├─ topology.rs         Shallow folder map → <repo_topology>
│  ├─ symbol_enrichment.rs rustdoc-JSON sidecar (gaviero-cli --graph --enrich)
│  └─ symbol_search.rs    Semantic search over enriched symbols
├─ acp/                   Legacy Claude NDJSON transport
│  ├─ session.rs / protocol.rs / client.rs / factory.rs
├─ agent_session/         V9 AgentSession trait + impls
│  ├─ mod.rs              Turn, TransportContext, build_turn, LegacyAgentSession
│  ├─ claude.rs / codex_exec.rs / codex_app_server.rs / cursor.rs / ollama.rs
│  ├─ registry.rs         SessionConstruction by ProviderProfile
│  ├─ tool_agent/         In-process API harness (deepseek: today)
│  │  ├─ mod.rs / client.rs / config.rs / agent_loop.rs / policy.rs
│  │  ├─ replay.rs / snapshot.rs / swarm.rs
│  │  └─ tools/           read / write / bash / glob / grep
│  └─ agent_client_protocol/  Pattern D — ACP client (`dsh:`)
├─ context_planner/       Bootstrap / delta / replay → PlannerSelections
├─ mcp/                   In-process MCP server
│  ├─ server.rs           spawn_mcp_server, GavieroMcpServer
│  ├─ signal.rs           MemorySignalSink — the one write-adjacent seam
│  ├─ tools.rs            Ten tools (see MCP below)
│  ├─ probe.rs            ProbeLedger + memory_ping receipts
│  ├─ transport.rs        McpEndpoint (Unix socket / Windows named pipe)
│  ├─ http.rs             Loopback streamable-HTTP listener
│  ├─ endpoint_file.rs    mcp-endpoint.json for shim --resolve
│  ├─ reach_probe.rs      Nested MCP reach (memory_ping)
│  ├─ user_scope.rs       claude/codex user-scope registration (CLI flags)
│  ├─ config_synth.rs     Per-worktree .mcp.json / .codex/ / .cursor/
│  ├─ preflight.rs        Shim PATH + URL checks
│  ├─ telemetry_sink.rs   NDJSON call metrics (--mcp-stats)
│  ├─ payload.rs          Estimated payload tokens (stats + outline budget)
│  ├─ resolver.rs         Endpoint / config path resolution
│  ├─ external_memory.rs  Competing memory-MCP detection
│  └─ observer.rs         McpToolCallObserver
├─ memory/                Multi-DB store, writer task, RRF retrieval, eval
├─ swarm/                 Six-phase pipeline + backends
│  └─ backend/            AgentBackend + UnifiedStreamEvent
│     ├─ claude_code.rs / codex.rs / cursor.rs / ollama.rs
│     ├─ deepseek.rs      DeepseekBackend → tool_agent
│     ├─ dsh.rs           DshBackend → AcpClientSession
│     ├─ mock.rs / executor.rs / runner.rs / shared.rs
│     └─ mod.rs           BackendConfig (incl. Deepseek)
├─ iteration/             IterationEngine (retry, BestOfN, TDD)
└─ validation_gate/       TreeSitterGate + CargoCheckGate
```

---

## Abstractions

### `FileScope` ([`types.rs`](src/types.rs))

`owned_paths`, `read_only_paths`, `interface_contracts`. Matched by [`path_pattern::matches`](src/path_pattern.rs); pairwise overlap via [`patterns_overlap`](src/path_pattern.rs) (glob-disjoint siblings ok; prefix/subdir overlaps flagged).

### `WorkUnit` / `CompiledPlan`

- [`WorkUnit`](src/swarm/models.rs) — scope, client/tier/privacy, retries, memory routing, context expansion.
- [`CompiledPlan`](src/swarm/plan.rs) — `DiGraph<PlanNode, DependencyEdge>` + iteration / verification / loop config. `work_units_ordered()`, `hash()` for checkpoints. Produced by `gaviero-dsl` or [`coordinator::plan_coordinated`](src/swarm/coordinator.rs); consumed by [`pipeline::execute`](src/swarm/pipeline.rs).

### `AgentBackend` + `UnifiedStreamEvent` ([`swarm/backend/mod.rs`](src/swarm/backend/mod.rs))

```rust
async fn stream_completion(&self, req: CompletionRequest)
    -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;
```

Events: `TextDelta | ThinkingDelta | ToolCallStart/Delta/End | FileBlock | PathsModified | Usage | Error | Done`.

[`BackendConfig`](src/swarm/backend/mod.rs): `ClaudeCode | Codex | Cursor | Ollama | Deepseek | Dsh | Custom` (Custom unimplemented). Materialized by [`create_backend`](src/swarm/backend/mod.rs).

**Model spec** ([`shared.rs`](src/swarm/backend/shared.rs)):

| Spec | Backend |
|---|---|
| `claude:<name>` | `ClaudeCodeBackend` |
| `codex:<name>` | `CodexBackend` |
| `cursor:<name>` | `CursorBackend` |
| `ollama:<name>` / `local:<name>` | `OllamaStreamBackend` |
| `deepseek:<name>` | `DeepseekBackend` → [`tool_agent`](src/agent_session/tool_agent) |
| `dsh:<name>` | `DshBackend` → [`AcpClientSession`](src/agent_session/agent_client_protocol) |

[`validate_model_spec`](src/swarm/backend/shared.rs) rejects bare names. Prefixes: `SUPPORTED_PROVIDER_PREFIXES` = `claude`, `codex`, `cursor`, `ollama`, `local`, `deepseek`, `dsh`. DeepSeek ids: `DEEPSEEK_API_MODELS` (`deepseek-flash`, `deepseek-v4-pro`, `deepseek-v4-flash`) for both `deepseek:` and `dsh:`.

### `AgentSession` + `Turn` ([`agent_session/mod.rs`](src/agent_session/mod.rs))

[`build_turn`](src/agent_session/mod.rs) lifts [`PlannerSelections`](src/context_planner/types.rs) into a `Turn`. [`registry::create_session`](src/agent_session/registry.rs) routes by `ProviderProfile`:

| Provider | Continuity | Session |
|---|---|---|
| `claude` | NativeResume | `ClaudeSession` |
| `cursor` | NativeResume | `CursorSession` |
| `codex` | StatelessReplay | `CodexExecSession` |
| `codex-app` | ProcessBound | `CodexAppServerSession` |
| `ollama` / `local` | StatelessReplay | `OllamaSession` |
| `deepseek` | StatelessReplay | `ToolAgentSession` |
| `dsh` | ProcessBound | `AcpClientSession` |

Writes, TUI chat: every provider writes during the turn (`AgentOptions::host_capture`) and [`turn_capture`](src/turn_capture) records the turn's change set for the mandatory post-turn review. Writes, swarm / CLI: native edit tools (Claude/Codex/Cursor) in the worktree, ACP `fs/write_text_file` (`dsh:`), or `<file>` blocks (Ollama) → [`WriteGatePipeline`](src/write_gate.rs).

### Memory / MCP / observers

- [`MemoryStores`](src/memory/stores.rs) + [`WriterHandle`](src/memory/writer.rs) — multi-DB; single writer task.
- [`GavieroMcpServer`](src/mcp/server.rs) — ten tools (nine read-only including `memory_ping` and `symbol_refs` + write-adjacent `memory_flag`); **no `WriterHandle`** — `memory_flag` signals through [`mcp/signal.rs`](src/mcp/signal.rs) into the writer task. `memory_ping` records only on an in-memory [`ProbeLedger`](src/mcp/probe.rs). Loopback HTTP: [`mcp/http.rs`](src/mcp/http.rs). Shim lookup: [`mcp/endpoint_file.rs`](src/mcp/endpoint_file.rs). Lean surface: `mcp.gavieroServer.exposedTools` (default all ten; lean preset `memory_search`/`memory_get`/`memory_ping`).
- Observers in [`observer.rs`](src/observer.rs) / [`memory/observer.rs`](src/memory/observer.rs) / [`mcp/observer.rs`](src/mcp/observer.rs).

---

## Data Flow

### Swarm (6 phases) — [`pipeline::execute`](src/swarm/pipeline.rs)

```
VALIDATE   pairwise scope overlap, Kahn topo-sort, SwarmContextBundle
EXECUTE    worktree + MCP config synth → IterationEngine →
             ContextPlanner::plan → Turn → AgentSession::send_turn →
             FileBlock / tool writes → WriteGatePipeline
MERGE      git merge --no-ff; MergeResolver on conflict
VERIFY     Structural | DiffReview | TestSuite | Combined
CLEANUP    teardown worktrees / gaviero/* branches / MCP configs
CONSOLIDATE Consolidator + TierStats → memory
```

### Turn capture (TUI chat)

```
TurnCapture::begin   stat walk (.gitignore + files.exclude, no git) → re-hash changed → blobs ≤ 8 MiB
  session runs       providers write freely (host_capture); host writes → HostWriteLedger
  session closed     (also on cancel: the select drops the session first)
TurnCapture::end     stat walk → hash suspects → TurnChangeSet {added|modified|deleted, before/after blobs}
                     − ledger-attributed host writes; overlap marks vs concurrent turns
  auto_revert_sensitive → save_pending → history files_changed → TURN REVIEW (conversation blocked)
TurnCapture::resolve keep | revert_file | revert_hunks (byte-exact, drift-guarded) → archive → gc (last 5)
```

No `Mutex` is held across the walk, hashing, or any file write.

### Write proposal (swarm / CLI)

```
UnifiedStreamEvent::FileBlock (or Cursor snapshot+revert / tool_agent PathsModified)
  → BRIEF LOCK  scope_enforcer / path_pattern
  → NO LOCK     diff_engine::compute_hunks + tree_sitter::enrich_hunks
  → BRIEF LOCK  WriteGatePipeline::insert_proposal (Interactive|AutoAccept|Deferred|RejectAll)
  → NO LOCK     fs::write when finalized
```

### Memory write + retrieve

```
WRITE     WriterHandle::send → writer task: embed (no lock) → brief DB lock → optional ack (ACK_TIMEOUT_MS, 30s)
RETRIEVE  retrieve_ranked: embed → MemoryStores::search_scoped (merged RRF default | cascade kill-switch)
          → score → optional rerank → injection_manifests
          Chat/swarm first-turn injection is a constitution filter (user-authored
          + Decision/Convention/Invariant/Preference); the archive is MCP `memory_search`.
```

DBs: global `~/.config/gaviero/memory.db`; workspace+run `<workspace>/.gaviero/memory.db`; repo+module `<folder>/.gaviero/memory.db`.

### MCP

[`spawn_mcp_server`](src/mcp/server.rs) binds [`McpEndpoint`](src/mcp/transport.rs). Subprocess agents use `gaviero-mcp-shim`. Tools ([`tools.rs`](src/mcp/tools.rs)): `memory_search`, `memory_get`, `memory_ping`, `blast_radius`, `node_doc`, `repo_outline`, `symbol_refs`, `symbol_search`, `symbol_doc` (last two gated by `repoMap.symbolEnrichment.enabled`).

Graph-tool output contracts:
- **`blast_radius`** returns `{mode, direction, nodes, not_shown, truncated}`.
  - `direction` is `dependents` (default: files that use the seeds), `dependencies` (files the seeds use) or `both`; anything else is `invalid_params`.
  - The files come from a breadth-first walk of the cross-file edges the mode includes ([`walk_files`](src/repo_map/mod.rs)). For `dependents` this is the same set as the impact CTE.
  - The nodes are `changed` rows, then up to `limit` ranked rows (default 40, max 200, 0 allowed), then `test_gap` rows.
  - Ranked rows are ordered by full-precision score with path as tie-break; scores are rounded to 3 decimals after ranking. The score is mode-weighted personalized PageRank on the graph oriented the way the walk goes, so the seeds' rank flows to what the walk reaches.
  - Each ranked row carries `depth` (hops from the nearest seed) and `via`, the edge that first reached it: `"<Kind> <source> -> <target> L<line>"`. The line is 1-based in the referencing file. The side in the row's own file is a bare symbol name, the other side a folder-relative qualified name.
  - `relation` appears only on changed and test-gap rows, which carry no `depth` or `via`. No row carries `qualified_name`.
- **`symbol_refs`** returns `{qualified_name, refs, not_shown, truncated}` for one symbol `qualified_name` (`<path>::<name>`, as `node_doc` gives it). A bare path is `invalid_params` (use `blast_radius`), and so is an unknown symbol.
  - `direction`: `callers`, `callees` or `both` (default).
  - Each ref has `role` (`caller` / `callee`), the other end's `qualified_name` (a file path for a top-level reference), `path`, a 1-based `line` (the call site for callers, the definition for callees), `kind` (`Calls` / `Imports` / `Implements`), and `test: true` when a caller is test code.
  - Callers come first, then callees. `limit` defaults to 40 (max 200, 0 allowed) and applies to the combined list. An absolute path selects its workspace folder. Paths of a non-primary folder are shown absolute.
- **`node_doc`** symbols carry `qualified_name`, a 1-based `line` (null when unknown) and `signature` only when it differs from the name. There is no legacy `signatures` list. An empty result carries `status` (an extraction outcome, `unknown` or `not_indexed`) and `note`.
- **`repo_outline`** entries are `{rendered}` in rank order. `token_estimate` is the estimate (`ceil(bytes/4)`, [`payload.rs`](src/mcp/payload.rs)) of the whole compact response and never exceeds the clamped `token_cap`.
- **`repo_outline` `seed_paths`** omitted, empty or `.` means no personalization: an unseeded global ranking with no `OWN` lines.

**Telemetry:** `ServerHandler::call_tool` and `call_tool_in_process` each emit exactly one entry per invocation, failures included (error set, null output). The `*_impl` cores never emit.

**Extraction** ([`repo_map/extraction.rs`](src/repo_map/extraction.rs)) is one policy shared by `RepoMap` and the graph builder:
- Limits: a 1 MB read limit, a 400 KB symbol-parse cap, a 2 s deadline, and exclusion of files with an `@generated` header (symbols and edges).
- The outcome of each path is persisted in the graph's `file_extraction` table.
- Changing `EXTRACTION_POLICY_VERSION` re-extracts every file on the next build.

**Reference resolution** ([`repo_map/resolve.rs`](src/repo_map/resolve.rs)) turns extracted references into edges. Rust, TypeScript and Python each have one structured extractor in [`edges.rs`](src/repo_map/edges.rs). It keeps the written path, the call style (bare / scoped / method), import bindings and re-exports. Other languages keep bare-name resolution.

- **Rust.**
  - A crate/module map is built from `Cargo.toml` files and the `src/` tree.
  - Each file gets an import table from its `use` / `extern crate`.
  - `pub use` re-exports are followed (stored in the graph's `reexports` table, at most 4 hops).
  - Symbol owners are recorded (`nodes.owner`: `impl:Type`, `impl:Type:Trait`, `trait:Name`).
  - A scoped call resolves through its path. `Type::m` goes to the type's `impl` methods.
  - A bare call resolves through its import binding, then a local definition, then glob imports.
  - A method call goes to the trait's file when a trait declaring it is in scope, so `dyn Trait` impact lands on the trait. Otherwise it goes to `impl` methods of the types the file imports or defines.
- **TypeScript.** Relative specifiers resolve to files; `export … from` is followed; namespace and named imports are bound.
- **Python.** Modules come from the package tree. Relative imports are supported, and `mod.f()` resolves through module bindings.
- **Receiver typing (Rust method calls).** The walk ([`rust_walk.rs`](src/repo_map/rust_walk.rs)) records each method call's receiver as written ([`types::ReceiverExpr`](src/repo_map/types.rs)):
  - `self` (the impl type), parameters and annotated `let`s;
  - `let x = …` (the initialiser substituted, ≤ 4 levels), field chains, method results, `T::new(…)`, `?`, `.await`, literals.

  Pattern bindings and unannotated closure parameters are unknown. The walk also persists per-file **type facts**:
  - `type_fields`, `fn_returns`, `type_impls`, `type_aliases`;
  - `rust_imports`: every `use` binding, because a type written in another file is resolved with *that* file's imports;
  - `type_methods`: each impl / trait block's methods, which survive the node collapse of same-named methods in one file. [`resolve/receiver.rs`](src/repo_map/resolve/receiver.rs) evaluates the receiver to a workspace type, an external type, trait bounds or unknown:
  - `Box` / `Arc` / `Rc` / `&` / lock guards are dereferenced.
  - A small std transform table covers `unwrap`, `expect`, `?`, `lock`, `read` / `write`, `borrow_mut`, `get`, `first` and `clone`.
  - Aliases are expanded.

  Then the method is looked up:
  - **Workspace type:** inherent → trait impl → trait default; none (a derived or std trait method) → no edge.
  - **Trait bounds** (`&dyn T`, `impl T`, `G: T`): the trait's file.
  - **External type:** no edge, unless a workspace extension trait in scope is implemented for it.
  - **Unknown:** the name-based steps above.
- **Fallback.** The last fallback everywhere is the name: definitions in files of the same language, free functions only for `foo()` and methods only for `.foo()`. A `.foo()` whose receiver is unknown and whose name std also defines (`ok`, `send`, `elapsed`, … — [`std_methods.txt`](src/repo_map/std_methods.txt), generated from the toolchain's rustdoc HTML by `python -I research/gen_std_methods.py`; regenerate after a toolchain update, `--check` verifies) gets no edge.
- **Filters.** Production files never target test files. A reference that resolves only to its own file's definition (`Outcome::Local`) gets one same-file edge, unless it is a recursive call. File-level consumers (`file_edges`, the impact walk, ranking) skip same-file edges.
- **Ambiguity is dropped, not fanned out.** When candidates span more than `RESOLVE_MAX_TARGET_FILES` files (1, chosen on labelled samples), the reference gets no edge. `BuildResult.resolve` counts:
  - resolution by step;
  - local, unresolved and dropped references;
  - deliberate no-edge outcomes (`external_receiver`, `typed_no_method`, `std_name_unknown_receiver`);
  - receiver shapes.
- **Staleness.** When any file's definitions, re-exports or type facts (field / return types, impl headers, aliases, `use` bindings, impl / trait methods) change, every unchanged structured-language file is re-resolved in the same build (about 1.7 s on this repo). A body-only or comment-only edit re-resolves nothing else.
- **Known limits.**
  - Type inference is syntactic only: no tuple or struct destructuring, no iterator element types (`for x in v`), no unannotated closure parameters, no generic substitution through user types, no workspace `Deref` impls. Such receivers fall back to the name-based steps.
  - Two methods of one name in one file share a node (the `qualified_name` format); method lookups use the `type_methods` facts instead, but the edge's target node is the shared one.
  - Macros are not expanded, and `#[path]` is not modelled. Inline `mod x { }` items stay in their file.
  - A function-scoped `use` counts as file-wide.
  - Same-named symbols in one file share one node, which keeps the range of the last one stored. A reference inside an earlier same-named method falls outside that range, so it is attributed to an enclosing symbol or to the file. An inline test helper never replaces a same-named production node.

**Symbol-level edges** ([`repo_map/spans.rs`](src/repo_map/spans.rs)):
- Nodes store `line_start` and `line_end`.
- A reference edge's `source_qn` is the innermost stored symbol whose range contains the reference, or the file (its File node) at top level. Ranges come from the stored nodes, so full builds and `update_paths` agree.
- `file_edges` counts each (file, target, kind) once, so per-caller rows do not change file-level weights. `symbol_refs` reads callers and callees from these rows.
- Rust inline test code is stored as `Test` nodes and emits `TestOf` to the non-test code it references, same-file included, so a file covered only by its own `#[cfg(test)]` tests is no longer a test gap. Inline test code is a function with a test attribute (`#[test]`, `#[tokio::test]`, …) or any function in a `#[cfg(test)]` module. A file's own non-`TestOf` edges never count as test coverage by path.

### Two-layer graph context

First turn: `<repo_topology>` ([`topology::build_folder_topology`](src/repo_map/topology.rs)) + `<repo_outline>`. `/lite` keeps topology only.

`<repo_outline>` is PageRank over the code graph's file edges ([`RepoMap::from_store`](src/repo_map/mod.rs) projects the code graph, edges from `GraphStore::file_edges`). Each edge counts forward (referencing → referenced file) at full weight and in reverse at `REVERSE_EDGE_FACTOR` (0.5). It lists only files the code graph indexes, i.e. those with a tree-sitter grammar. A `.` or empty seed is an unseeded global ranking.

**Live graph** ([`repo_map/service.rs`](src/repo_map/service.rs)). Each workspace folder has one `GraphService`, whose writer thread is the only writer of `<folder>/.gaviero/code_graph.db` in the process.
- **Updates.** File-watcher paths are coalesced (300 ms of quiet, 2 s cap) and applied with `graph_builder::update_paths` in one transaction. More than 1000 dirty paths, a `rescan`, or the startup pass trigger a full incremental scan instead.
- **Publishing.** Every batch commits to disk, then publishes a new `RepoMap` snapshot.
- **Readers.** Chat injection (TUI `App::graph_services`) and every MCP graph tool read the same services. Readers open fresh WAL connections and wait (`flush`) for the pending batch. Paths under dot directories, such as gaviero's own `.gaviero/` writes, never trigger a batch.
- **CLI.** gaviero-cli has no watcher: its MCP graph is scanned once per process.

---

## Concurrency

| Component | Primitive | Rule |
|---|---|---|
| WriteGatePipeline | `tokio::sync::Mutex` | No lock across diff / parse / I/O |
| MemoryStore (×N) | `Mutex<Connection>` per DB | Embed outside lock; writes via writer task |
| MCP graph cache | `Mutex<GraphStore>` | Lazy init; serializes blast_radius |
| Writer task | `mpsc` + optional oneshot | Single consumer |
| AgentBus | `broadcast` | Lock-free |
| Parallel agents | `Semaphore` | Bounded per tier |

**Never hold a Mutex across `.await`, tree-sitter parse, or `fs` I/O.** Enforced in [`memory/writer.rs`](src/memory/writer.rs) via `#![deny(clippy::await_holding_lock)]`.

---

## Error Handling

| Error | Handling |
|---|---|
| Scope violation | Reject proposal; observer; no retry |
| Agent failure | `AgentStatus::Failed` → escalate / replan stub |
| Validation gate | Corrective feedback → retry |
| Merge conflict | Claude resolver or user |
| Memory init | Non-fatal `Option<Arc<MemoryStores>>` |
| C1 migration pending | Refuse open until consent |
| MCP bind failure | Log; agents fall back |
| Cursor argv overflow | Spill to tempfile (Windows 30 KB / Unix 96 KB) |

---

## API

```rust
// crates/gaviero-core/src/lib.rs — 28 pub mods
pub mod acp;
pub mod agent_session;   // + tool_agent (deepseek:) + agent_client_protocol (dsh:)
pub mod context_planner;
pub mod diff_engine;
pub mod git;
pub mod git_conflict;
pub mod history;         // per-turn NDJSON log (TUI HISTORY panel, gaviero-cli --history)
pub mod indent;
pub mod iteration;
pub mod mcp;
pub mod memory;
pub mod observer;
pub mod path_pattern;
pub mod query_loader;
pub mod repo_map;        // + topology, symbol_enrichment, symbol_search
pub mod scope_enforcer;
pub mod session_state;
pub mod skills;
pub mod swarm;           // backends incl. Deepseek
pub mod terminal;
pub mod tree_sitter;
pub mod turn_capture;    // host-side, git-free per-turn change capture + revert (TUI chat)
pub mod types;
pub mod util;
pub mod validation_gate;
pub mod workspace;
pub mod write_gate;

pub use ::tree_sitter::{Language, Parser, Tree, Node, Query, QueryCursor, Point, InputEdit};
```

Hard constraints (Write Gate, MCP read-only, explicit `WriteScope`, `provider:model` specs, no UI/DSL deps): see [CLAUDE.md](CLAUDE.md) Rules.
