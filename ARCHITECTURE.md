# Gaviero — Architecture

> Terminal code editor for AI agent orchestration, written in Rust 2024.

**Binaries:** `Gaviero` (TUI editor), `Gaviero-cli` (headless swarm runner)
**Platform:** Linux (primary), any POSIX with a modern terminal
**Build:** `cargo build` from workspace root — no external tooling

---

## 1. Crate Topology

```
                 ┌──────────────┐     ┌──────────────┐
                 │  Gaviero-tui │     │  Gaviero-cli │
                 │  (binary)    │     │  (binary)    │
                 └──────┬───────┘     └──────┬───────┘
                        │                    │
                        ▼                    ▼
                 ┌─────────────────────────────────┐
                 │          Gaviero-core            │
                 │          (library)               │
                 └─────────────────────────────────┘
```

**Separation rule:** All pipeline logic lives in `Gaviero-core`. The TUI crate contains only rendering and input handling. The CLI crate contains only argument parsing and observer implementations. Core can be tested without any UI dependency.

| Crate | Role | Key dependencies |
|---|---|---|
| `Gaviero-core` | Logic: write gate, diff, tree-sitter, ACP, swarm, memory, git | tokio, tree-sitter (0.24) + 13 langs, git2, rusqlite, ort, similar, ropey |
| `Gaviero-tui` | TUI editor binary | Gaviero-core, ratatui (0.30), crossterm (0.28), portable-pty, vt100, notify |
| `Gaviero-cli` | Headless swarm runner | Gaviero-core, clap, tokio |

Tree-sitter types are re-exported from `Gaviero-core::lib.rs` (`Language`, `Tree`, `Parser`, `Query`, `QueryCursor`). Downstream crates never depend on `tree-sitter` directly.

---

## 2. Module Map

### Gaviero-core/src/

```
lib.rs                      Re-exports, module declarations
types.rs                    FileScope, DiffHunk, HunkType, WriteProposal, StructuralHunk, NodeInfo
workspace.rs                Workspace model, WorkspaceFolder, settings cascade
session_state.rs            SessionState, TabState, PanelState, StoredConversation
tree_sitter.rs              Language registry (13 langs), enrich_hunks(), extract_outline()
diff_engine.rs              compute_hunks() — similar crate wrapper
write_gate.rs               WriteGatePipeline, WriteMode, proposal management
observer.rs                 WriteGateObserver, AcpObserver, SwarmObserver trait definitions
git.rs                      GitRepo (git2 wrapper), WorktreeManager, FileStatus
query_loader.rs             Tree-sitter .scm file discovery (env var → exe dir → cwd → bundled)
acp/
  session.rs                AcpSession: spawn Claude subprocess, NDJSON read/write
  protocol.rs               StreamEvent enum, NDJSON parsing
  client.rs                 AcpPipeline: prompt enrichment, file block detection, proposal routing
swarm/
  models.rs                 WorkUnit, AgentManifest, AgentStatus, SwarmResult, MergeResult
  validation.rs             validate_scopes() (overlap detection), dependency_tiers() (Kahn's)
  runner.rs                 AgentRunner: single WorkUnit execution
  pipeline.rs               SwarmPipeline: tier orchestration, parallel execution, merge
  merge.rs                  MergeResolver: git merge + Claude-powered conflict resolution
  planner.rs                TaskPlanner: natural language → WorkUnit decomposition
  bus.rs                    AgentBus: broadcast + targeted inter-agent messaging
memory/
  embedder.rs               Embedder trait (embed, embed_batch, dimensions, model_id)
  onnx_embedder.rs          OnnxEmbedder: ort + tokenizers, mean pooling, L2 normalization
  store.rs                  MemoryStore: async SQLite wrapper, brute-force cosine similarity
  schema.rs                 SQL DDL, migrations (memories table)
  model_manager.rs          ONNX model download + caching (~/.cache/Gaviero/models/)
indent/
  mod.rs                    compute_indent() entry point
  treesitter.rs             Tree-sitter-based indent
  heuristic.rs              Hybrid indent (relative delta)
  bracket.rs                Bracket-counting fallback
  captures.rs               Tree-sitter capture processing
  predicates.rs             Indent rule predicates
  config.rs                 Indent configuration
```

### Gaviero-tui/src/

```
main.rs                     Entry point, terminal setup, event loop
app.rs                      App state, layout rendering, focus management
event.rs                    Event enum, EventLoop, observer implementations
keymap.rs                   Keybinding definitions
theme.rs                    Theme (TOML) → ratatui::Style mapping
editor/
  buffer.rs                 Ropey buffer, Cursor, Transaction, undo/redo
  view.rs                   Editor rendering: gutter, syntax highlights, scroll, cursor
  diff_overlay.rs           Diff review mode: inline hunks, accept/reject per hunk
  highlight.rs              Tree-sitter highlight query runner → Vec<StyledSpan>
  markdown.rs               Markdown rendering for chat display
panels/
  file_tree.rs              Multi-root file browser, git + proposal decorations
  agent_chat.rs             Chat input, streaming response, conversation management
  swarm_dashboard.rs        Agent status table with tier/phase labels
  git_panel.rs              Staging area, commit, branch selection
  terminal.rs               Embedded PTY (portable-pty + vt100 + tui-term)
  status_bar.rs             Mode, file, branch, agent status indicators
  search.rs                 File/project search
widgets/
  tabs.rs                   Tab bar with close indicators
  scrollbar.rs              Custom scrollbar widget
  input.rs                  Multi-line text input
  borders.rs                Resizable split borders
```

### Gaviero-cli/src/

```
main.rs                     clap CLI, CliAcpObserver, CliSwarmObserver, SwarmPipeline launcher
```

---

## 3. Core Abstractions

### FileScope

Defines an agent's permission boundary over the filesystem.

```
FileScope {
    owned_paths: Vec<PathBuf>     Files/dirs the agent may write
    read_only: Vec<PathBuf>       Files/dirs the agent may read but not write
    interface_contracts: Vec<String>  API contracts the agent must preserve
}
```

Used by: Write Gate (scope validation), Swarm (overlap detection), Agent Runner (prompt enrichment).

### WriteProposal

A set of proposed file changes pending user review.

```
WriteProposal {
    id: u64
    source: String               Agent ID or "user"
    path: PathBuf                Target file
    hunks: Vec<StructuralHunk>   Diff hunks with AST context
    status: ProposalStatus       Pending | PartiallyAccepted | Accepted | Rejected
}
```

### DiffHunk / StructuralHunk

```
DiffHunk {
    start_line, end_line: usize   0-indexed line range
    original: String              Original text
    proposed: String              New text
    hunk_type: HunkType           Added | Removed | Modified
}

StructuralHunk = DiffHunk + {
    enclosing_node: Option<NodeInfo>   AST context (e.g. "function parse_config")
    description: String                Human-readable ("Modify lines 10-15 in function 'parse_config'")
    status: HunkStatus                 Pending | Accepted | Rejected
}
```

### WorkUnit

A single task assigned to one swarm agent.

```
WorkUnit {
    id: String
    description: String
    scope: FileScope
    depends_on: Vec<String>       IDs of prerequisite WorkUnits
    backend: AgentBackend         ClaudeCode | Codex | Custom
    model: Option<String>         Per-unit model override
}
```

### SwarmResult

Aggregate outcome of a swarm execution.

```
SwarmResult {
    manifests: Vec<AgentManifest>    Per-agent results
    merge_results: Vec<MergeResult>  Per-branch merge outcomes
    success: bool
}
```

---

## 4. Event Architecture

A single `tokio::sync::mpsc::unbounded_channel<Event>` carries all external events to the TUI main loop. No background task mutates application state directly.

### Event Sources

| Source | Events produced | Mechanism |
|---|---|---|
| Crossterm reader | `Key`, `Mouse`, `Paste`, `Resize` | Blocking thread → channel |
| File watcher (notify) | `FileChanged`, `FileTreeChanged` | Callback → channel |
| Tick timer | `Tick` (~33ms, ~30fps) | tokio::interval → channel |
| WriteGateObserver | `ProposalCreated`, `ProposalUpdated`, `ProposalFinalized` | Observer trait impl |
| AcpObserver | `StreamChunk`, `ToolCallStarted`, `MessageComplete` | Observer trait impl |
| SwarmObserver | `SwarmPhaseChanged`, `SwarmAgentStateChanged`, `SwarmTierStarted`, `SwarmMergeConflict`, `SwarmCompleted` | Observer trait impl |

### Observer Bridge Pattern

Three observer traits in `Gaviero-core::observer` define callbacks that core pipelines invoke. The TUI crate implements each trait with a struct holding a clone of the event sender:

```
                    Gaviero-core                         Gaviero-tui
              ┌─────────────────────┐            ┌──────────────────────┐
              │  WriteGateObserver  │◄───────────│  TuiWriteGateObserver│
              │  AcpObserver        │◄───────────│  TuiAcpObserver      │
              │  SwarmObserver      │◄───────────│  TuiSwarmObserver    │
              └─────────────────────┘            └──────────┬───────────┘
                                                            │
                                                   sends Event to channel
                                                            │
                                                            ▼
                                                     App::handle_event()
```

### Main Loop

```
loop {
    terminal.draw(|frame| app.render(frame));
    event = event_rx.recv().await;
    app.handle_event(event);
    if app.should_quit { break; }
}
```

---

## 5. Data Flow: Agent Write Proposal

This is the central pipeline — every agent file write passes through it.

```
  Agent (AcpSession)
    │
    │  <file path="src/foo.rs">...content...</file>   (detected in NDJSON stream)
    │
    ▼
  AcpPipeline::propose_write(path, content)
    │
    ├─ 1. BRIEF LOCK: write_gate.is_scope_allowed(agent_id, path)?
    │     Release lock
    │
    ├─ 2. NO LOCK:
    │     original = fs::read_to_string(path)
    │     hunks = diff_engine::compute_hunks(original, content)
    │     structural = tree_sitter::enrich_hunks(hunks, original, language)
    │     proposal = WriteProposal { hunks: structural, status: Pending }
    │
    ├─ 3. BRIEF LOCK: write_gate.insert_proposal(proposal)
    │     ├─ Interactive → queue, fire on_proposal_created() → TUI shows diff overlay
    │     ├─ AutoAccept → accept all, return Some((path, content))
    │     └─ RejectAll  → discard silently
    │     Release lock
    │
    └─ 4. NO LOCK: if AutoAccept, write content to disk
```

**Lock discipline:** The `WriteGatePipeline` Mutex is never held across I/O, tree-sitter parsing, or diff computation. Locks are held only for brief HashMap operations.

---

## 6. Data Flow: Swarm Execution

```
  SwarmPipeline::execute(work_units, config)
    │
    ├─ Phase 1: VALIDATE
    │   validate_scopes() → check no owned_path overlaps (O(n^2) pairwise)
    │   dependency_tiers() → Kahn's topological sort → Vec<Vec<WorkUnitId>>
    │
    ├─ Phase 2: EXECUTE (per tier, sequentially)
    │   │
    │   │  Tier N: [A, B, C]  (can run in parallel)
    │   │
    │   ├─ For each WorkUnit (bounded by Semaphore):
    │   │   ├─ Provision git worktree (branch: Gaviero/{id})
    │   │   ├─ AgentRunner::run()
    │   │   │   ├─ Enrich prompt with memory context + scope clause
    │   │   │   ├─ Spawn AcpSession (WriteMode::AutoAccept)
    │   │   │   ├─ Stream NDJSON → propose_write for each file block
    │   │   │   └─ Return AgentManifest
    │   │   └─ Broadcast completion to AgentBus
    │   │
    │   └─ Collect all manifests for tier
    │
    └─ Phase 3: MERGE (if use_worktrees)
        For each successful agent branch:
          git merge --no-ff → main
          On conflict: MergeResolver queries Claude for resolution
        Return SwarmResult
```

---

## 7. Data Flow: Memory Search

```
  Caller (AgentRunner / AcpPipeline / MergeResolver)
    │
    │  memory.search(namespace, query_text, limit)
    │
    ├─ 1. NO LOCK: embedder.embed(query_text) → Vec<f32>   [CPU-heavy, ONNX inference]
    │
    ├─ 2. BRIEF LOCK: SELECT * FROM memories WHERE namespace = ?
    │     Release lock
    │
    └─ 3. NO LOCK: cosine_similarity(query_vec, stored_vec) for each row
         Sort by score, return top-K
```

Embedding computation (the expensive part) always runs outside the Mutex. The lock protects only SQLite I/O.

---

## 8. Write Gate

### Modes

| Mode | Behavior | Used by |
|---|---|---|
| `Interactive` | Queue proposals for TUI review (accept/reject per hunk) | Normal editor usage |
| `AutoAccept` | Accept all scope-valid writes immediately, write to disk | Swarm execution |
| `RejectAll` | Silently discard all proposals | Safety fallback |

### Proposal Lifecycle

```
Created (Pending) ──► User reviews hunks ──► Accepted / PartiallyAccepted / Rejected
                                                      │
                                                      ▼
                                              Finalized: assemble final
                                              content from accepted hunks,
                                              write to disk
```

### TUI Diff Overlay Keybinds

`]h`/`[h` navigate hunks, `a`/`r` accept/reject current hunk, `A`/`R` accept/reject all, `f` finalize, `q` exit review.

### Structural Awareness

Each hunk carries its enclosing AST node (function, class, struct, etc.), enabling `accept_node(proposal_id, "parse_config")` — accept all hunks within a named symbol.

---

## 9. ACP Integration

### Subprocess Model

Claude Code is spawned as a child process per agent session:

```
claude --print --output-format stream-json \
       --model <model> \
       --append-system-prompt <system> \
       --allowedTools Read,Glob,Grep \
       --add-dir <cwd>
```

The prompt is written to stdin (then closed). Stdout emits NDJSON (one JSON object per line).

### NDJSON Protocol (StreamEvent)

| Type | Content |
|---|---|
| `SystemInit` | `{ session_id, model }` |
| `ContentDelta` | Streaming text chunk |
| `ToolUseStart` | `{ tool_name, tool_use_id }` |
| `AssistantMessage` | Complete message `{ text, has_tool_use }` |
| `ResultEvent` | Final result `{ is_error, result_text, duration_ms, cost_usd }` |

### File Block Detection

The system prompt instructs the agent to output complete file content as:

```xml
<file path="relative/path">
...full file content...
</file>
```

`AcpPipeline` scans the accumulated response text incrementally for complete `<file>` blocks, extracts path + content, and routes each through `propose_write()`.

### Tool Restriction

Agents receive only read-only tools (`Read`, `Glob`, `Grep`). No `Edit`, `Write`, or `Bash`. All mutations flow through the Write Gate.

---

## 10. Swarm Orchestration

### Scope Validation

`validate_scopes()` performs O(n^2) pairwise comparison of `owned_paths` across all WorkUnits. Two agents cannot own the same file or overlapping directory prefixes.

### Dependency Tiers

`dependency_tiers()` applies Kahn's algorithm (topological sort) to the dependency graph defined by `WorkUnit.depends_on`. Returns `Vec<Vec<String>>` — each inner vec is a tier of units that can execute in parallel. Detects cycles → `CycleError`.

```
Example: A depends on nothing, B depends on A, C depends on A, D depends on B+C

Tiers: [[A], [B, C], [D]]
         │      │       │
         ▼      ▼       ▼
        Tier 0  Tier 1  Tier 2
       (serial) (parallel) (serial)
```

### Parallel Execution

Within a tier, agents run concurrently up to `config.max_parallel`, bounded by a `tokio::sync::Semaphore`.

### Git Worktree Isolation

When `use_worktrees = true`:
- Each agent gets its own branch: `Gaviero/{work_unit_id}`
- Each agent gets its own worktree: `.Gaviero/worktrees/{work_unit_id}/`
- `WorktreeManager` guarantees cleanup via `Drop`
- All git operations use the `git2` crate (never CLI)

### Merge Resolution

After all tiers complete, branches merge into main via `git merge --no-ff`. Conflicts trigger `MergeResolver` which reads conflict markers, queries Claude for resolution, writes the clean file, stages, and commits.

### Agent Bus

`AgentBus` provides inter-agent communication via tokio channels:
- `broadcast(from, content)` → all agents via `broadcast::channel`
- `send_to(from, to, content)` → targeted agent via per-agent `mpsc::UnboundedSender`

---

## 11. Memory System

### Architecture

```
┌─────────────────────────────┐
│        MemoryStore          │
│  ┌───────────┐ ┌─────────┐ │
│  │ rusqlite  │ │Embedder │ │
│  │ Connection│ │ (ONNX)  │ │
│  │ (Mutex)   │ │         │ │
│  └───────────┘ └─────────┘ │
└─────────────────────────────┘
```

- **Embedder trait:** `embed(text) → Vec<f32>`, `embed_batch(texts)`, `dimensions()`, `model_id()`
- **OnnxEmbedder:** Uses `ort` (ONNX Runtime) + `tokenizers` crate. CUDA-first with CPU fallback. L2-normalized output vectors.
- **Default model:** e5-small-v2 (384 dimensions). Alternative: EmbeddingGemma-300M (256-768 dims).
- **Search:** Brute-force cosine similarity (dot product on L2-normalized vectors). No ANN index.

### Schema

```sql
CREATE TABLE memories (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace  TEXT NOT NULL,
    key        TEXT NOT NULL,
    content    TEXT NOT NULL,
    embedding  BLOB,
    model_id   TEXT,
    metadata   TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    UNIQUE(namespace, key)
);
```

### Namespace Conventions

| Namespace pattern | Purpose |
|---|---|
| `project:{name}` | Project-level facts, outlines |
| `project:{name}:agents` | Agent execution summaries |
| `project:{name}:sessions` | Conversation memories |
| `global` | Cross-project knowledge |

### Integration Points

- **SwarmPipeline** stores outlines and results after execution
- **AcpPipeline** enriches prompts with semantic search results
- **MergeResolver** queries interface contracts during conflict resolution
- Memory is `Option<Arc<MemoryStore>>` — `None` before M3

---

## 12. Tree-Sitter Pipeline

### Language Registry

13 languages supported: Rust, Java, JavaScript, TypeScript, HTML, CSS, JSON, Bash, TOML, C, C++, LaTeX, (SQL/YAML via deps).

`language_for_extension(ext)` maps file extensions to `tree_sitter::Language` objects. Unknown extensions degrade gracefully — no parsing, plain diffs.

### Structural Enrichment

`enrich_hunks(hunks, original, language) → Vec<StructuralHunk>`:

1. Parse `original` with tree-sitter
2. For each hunk, walk up the AST from the hunk's line to find the enclosing named node (function, class, struct, enum, trait, impl, method)
3. Extract the node's identifier name
4. Generate human-readable description

### Syntax Highlighting

`highlight.rs` (TUI crate) runs tree-sitter highlight queries against the buffer's cached AST. Queries are loaded from `queries/{lang}/highlights.scm` (sourced from Helix, MIT licensed). Only the visible viewport range is processed.

### Indentation Engine

`indent/` module provides auto-indent via:
1. **Tree-sitter queries** (`queries/{lang}/indents.scm`) — preferred when available
2. **Hybrid heuristic** — relative delta from nearby line's actual indent
3. **Bracket counting** — fallback for unsupported languages

---

## 13. TUI Layout

Fixed 5-region layout (no floating windows):

```
┌──────────────────────────────────────────────────┐
│                    Tab Bar                        │
├────────┬──────────────────────────┬───────────────┤
│        │                          │               │
│  File  │        Editor            │  Side Panel   │
│  Tree  │     (center, largest)    │ (Agent Chat / │
│        │                          │  Swarm Dash / │
│        │                          │  Git Panel)   │
│        │                          │               │
├────────┴──────────────────────────┴───────────────┤
│                    Terminal                        │
├───────────────────────────────────────────────────┤
│                   Status Bar                      │
└───────────────────────────────────────────────────┘
```

### Focus Model

`Focus` enum: `Editor | FileTree | SidePanel | Terminal`. Tab cycles focus. Each panel handles its own keybindings when focused.

### Layout Presets

Six presets (number keys 1-6) configure column widths:

| Preset | File Tree | Editor | Side Panel |
|---|---|---|---|
| 1 (default) | 15% | 60% | 25% |
| 2 (editor only) | 0% | 100% | 0% |
| ... | varying proportions | | |

### Side Panel Modes

`SidePanelMode`: `AgentChat` (default), `SwarmDashboard`, `GitPanel`. Toggled via keybinds.

### Terminal Panel

Embedded shell via `portable-pty` (PTY allocation) + `vt100` (escape sequence parsing) + `tui-term` (rendering). Independent session with environment isolation (per-instance `HISTFILE`, stripped IDE env vars).

---

## 14. Concurrency Model

### Runtime

Single shared tokio runtime. All async work runs on this runtime. Initialized in `Gaviero-core/src/lib.rs`.

### Lock Discipline

| Rule | Rationale |
|---|---|
| Never hold `WriteGatePipeline` Mutex across I/O or parsing | Prevents pipeline stalls under concurrent agent writes |
| Never hold `MemoryStore` Mutex across embedding computation | ONNX inference is CPU-heavy (10-100ms) |
| Prefer pre-computing outside the lock, then brief lock for state update | Minimizes contention |
| Use channels (mpsc, broadcast) for cross-task communication | Lock-free on the critical path |

### Shared State Pattern

```rust
Arc<tokio::sync::Mutex<T>>  // for WriteGatePipeline, MemoryStore (SQLite connection)
Arc<dyn Observer>            // observer trait objects sent across tasks
mpsc::unbounded_channel      // event routing (single consumer: main loop)
broadcast::channel           // agent bus (multi-consumer)
Semaphore                    // parallel agent count bound
```

### Spawned Background Tasks

| Task | Lifetime | Mechanism |
|---|---|---|
| Crossterm event reader | App lifetime | Blocking thread via `tokio::task::spawn_blocking` |
| File watcher | App lifetime | `notify` crate callback → channel |
| Tick timer | App lifetime | `tokio::time::interval` |
| ACP session reader | Per-conversation | `tokio::spawn`, drops when session ends |
| Terminal PTY reader | Per-terminal | `tokio::spawn`, reads PTY output continuously |
| Swarm tier executor | Per-swarm run | `tokio::spawn` per agent, bounded by Semaphore |

---

## 15. Configuration

### Settings Cascade

Resolution order (first match wins):

1. `{folder}/.Gaviero/settings.json` — per-folder override
2. `.Gaviero-workspace` → `"settings"` — workspace-level
3. `~/.config/Gaviero/settings.json` — user-level
4. Hardcoded Rust defaults — fallback

For language-specific keys: if the current file is `.rs`, check `"[rust].{key}"` before `"{key}"` at each cascade level.

### Well-Known Settings Keys

| Category | Keys |
|---|---|
| Editor | `editor.tabSize`, `editor.insertSpaces`, `editor.formatOnSave` |
| Files | `files.exclude` (glob patterns) |
| Panels | `panels.fileTree.width`, `panels.sidePanel.width`, `panels.terminal.splitPercent` |
| Git | `git.treeAllowList` |
| Agent | `agent.model`, `agent.effort`, `agent.maxTokens` |
| Memory | `memory.namespace`, `memory.readNamespaces` |

### Theme Files

`themes/default.toml` maps highlight groups and UI elements to colors + attributes:

```toml
[syntax]
keyword = "fg=#569cd6 bold"
string  = "fg=#ce9178"
comment = "fg=#6a9955 italic"

[ui]
line_number        = "fg=#858585"
line_number.active = "fg=#ffffff bold"
cursor             = "bg=#ffffff fg=#000000"
selection          = "bg=#264f78"
diff.added         = "fg=#6a9955"
diff.removed       = "fg=#f48771"
```

### Query Files

`queries/{language}/highlights.scm` and `queries/{language}/indents.scm` — tree-sitter S-expression queries sourced from Helix (MIT). 13 language pairs.

### Session Persistence

`SessionState` (open tabs, cursor positions, panel visibility, conversation history) persists to platform data directory, keyed by FNV-1a hash of workspace path. Restored on startup, saved on quit.

---

## 16. Hard Constraints

These are architectural invariants. Do not violate them.

1. **Write Gate mandatory** — All agent file writes pass through `WriteGatePipeline`. No direct `fs::write` from agent code paths.
2. **git2 only** — Git operations use the `git2` crate. Never shell out to `git`.
3. **Tree-sitter for everything** — Structural analysis AND syntax highlighting. No regex-based highlighter.
4. **Mutex-wrapped SQLite** — `MemoryStore` wraps `rusqlite::Connection` in `tokio::sync::Mutex`. All DB methods are async. Shared as `Arc<MemoryStore>`.
5. **Single tokio runtime** — Initialized in `Gaviero-core/src/lib.rs`. All async work shares it.
6. **Core/TUI separation** — Pipeline logic in core. Rendering + input in TUI. Test core without TUI.
7. **Single event channel** — TUI receives all external events through one `mpsc::UnboundedReceiver<Event>`. No direct state mutation from background tasks.
8. **AutoAccept in swarm** — `WriteMode::AutoAccept` during swarm execution. User reviews aggregate result post-merge.
9. **No plugins** — Features compiled in. Configuration via settings files only.
10. **anyhow::Result** — All fallible operations. Custom error types only for structured validation data (`ScopeError`, `CycleError`).
