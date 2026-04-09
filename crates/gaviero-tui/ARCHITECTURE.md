# gaviero-tui — Architecture

The full-screen terminal UI binary. Holds no domain logic — all agent execution, swarm orchestration, write gating, memory, and git operations live in `gaviero-core`. The TUI's job is to render state, dispatch user input, and relay core events to UI updates via an mpsc channel.

---

## Module map

```
gaviero-tui/src/
├── main.rs              binary entry point: init terminal, build App, run event loop
├── app.rs               App struct (god object) + handle_event() dispatch (~5000 lines)
├── event.rs             Event enum (43+ variants), EventLoop, background task spawners
├── keymap.rs            KeyEvent → Action mapping; chord-prefix support
├── theme.rs             ~80 colour constants (One Dark); timing constants
│
├── editor/
│   ├── mod.rs           module re-exports
│   ├── buffer.rs        Buffer (Rope + tree-sitter + undo/redo), Cursor, Transaction
│   ├── view.rs          EditorView::render() — gutter, syntax highlighting, cursor, scrollbar
│   ├── diff_overlay.rs  Diff review mode: DiffSource, DiffReviewState, accept/reject per hunk
│   ├── highlight.rs     HighlightConfig, tree-sitter highlight query runner → Vec<StyledSpan>
│   └── markdown.rs      Markdown document rendering and editing
│
├── panels/
│   ├── mod.rs           module re-exports
│   ├── agent_chat.rs    AgentChatState, Conversation, slash-command handling, @file autocomplete
│   ├── swarm_dashboard.rs SwarmDashboardState, AgentEntry, activity log
│   ├── file_tree.rs     FileTreeState, lazy-loaded tree, dialogs (new/rename/delete)
│   ├── search.rs        SearchPanelState, live debounced workspace search
│   ├── git_panel.rs     GitPanelState, stage/unstage/commit/branch-picker
│   ├── status_bar.rs    context-sensitive bottom line renderer
│   ├── chat_markdown.rs ChatLine: markdown → ratatui StyledSpans renderer
│   └── terminal.rs      Terminal rendering (tui-term), TerminalSelectionState
│
└── widgets/
    ├── mod.rs           module re-exports
    ├── tabs.rs          TabBar widget with close indicators
    ├── scrollbar.rs     custom scrollbar widget
    ├── scroll_state.rs  ScrollState: shared scroll offset + selection for list panels
    ├── text_input.rs    TextInput: shared text editing with cursor, selection, undo/redo
    └── render_utils.rs  Shared rendering utilities
```

---

## App struct — state groups

`app.rs` uses a deliberate god-object pattern: all state in one `App` struct to avoid `Arc<Mutex<>>` lifetime entanglement across async tasks.

```
Workspace & buffers
  workspace                Workspace (settings, namespaces)
  buffers: Vec<Buffer>     open tabs
  active_buffer: usize

Focus & layout
  focus                    Editor | FileTree | SidePanel | Terminal
  left_panel               FileTree | Search | Review | Changes
  side_panel               AgentChat | SwarmDashboard | GitPanel
  panel_visible            { file_tree, side_panel, terminal }
  layout_presets           Alt+5..Alt+0 — tree%/editor%/side% splits
  layout: LayoutAreas      cached Rects for mouse hit-testing

Write gate & review
  write_gate               Arc<Mutex<WriteGatePipeline>>
  diff_review              single-file hunk overlay state
  batch_review             multi-file proposal list state

Agent chat
  chat_state               AgentChatState
  acp_tasks                HashMap<String, JoinHandle<()>>
  memory                   Option<Arc<MemoryStore>>

Swarm
  swarm_dashboard          SwarmDashboardState

Terminal
  terminal_manager         TerminalManager (PTY lifecycle)
  terminal_selection       TerminalSelectionState

Git
  git_repo                 Option<GitRepo>
  git_panel                GitPanelState
```

---

## Event system (`event.rs`)

### Architecture

```
Background producers ──mpsc::unbounded──► Event enum
                                               │
                              ┌────────────────▼──────────────────┐
                              │  Main loop (main.rs)              │
                              │  terminal.draw(app.render(frame)) │
                              │  event = rx.recv().await          │
                              │  app.handle_event(event)          │
                              │  drain up to 64 pending events    │
                              └───────────────────────────────────┘
```

All state mutations happen on the main thread. Background tasks only send `Event` values; they never touch `App` directly.

### Background producers

| Producer | Events |
|---|---|
| `spawn_crossterm_reader()` | `Key`, `Mouse`, `Resize`, `Paste` |
| `spawn_file_watcher()` | `FileChanged`, `FileTreeChanged` |
| `spawn_tick_timer()` (33 ms) | `Tick` |
| `spawn_terminal_bridge()` | `Terminal(TerminalEvent)` |
| Agent task (per chat turn) | `StreamChunk`, `ToolCallStarted`, `StreamingStatus`, `MessageComplete`, `FileProposalDeferred`, `AcpTaskCompleted` |
| Swarm task (`/swarm`, `/cswarm`) | `SwarmPhaseChanged`, `SwarmAgentStateChanged`, `SwarmTierStarted`, `SwarmCompleted`, `SwarmMergeConflict`, `SwarmCoordinationStarted`, `SwarmCoordinationComplete`, `SwarmTierDispatch`, `SwarmCostUpdate`, `SwarmDslPlanReady` |
| Write gate (via observer) | `ProposalCreated`, `ProposalUpdated`, `ProposalFinalized` |
| Memory init | `MemoryReady` |

### Event drain strategy

After processing the first event, the main loop calls `rx.try_recv()` up to **64 times** before redrawing. Prevents render rate from becoming the bottleneck during high-frequency streaming bursts (50+ `StreamChunk` events/s) without unbounded latency accumulation.

---

## Observer bridge

Three observer structs translate `gaviero-core` trait calls into `Event` sends:

### `TuiWriteGateObserver`
Carries `tx: UnboundedSender<Event>`.
```
on_proposal_created(p)      → Event::ProposalCreated(Box::new(p))
on_proposal_updated(id)     → Event::ProposalUpdated(id)
on_proposal_finalized(path) → Event::ProposalFinalized(path)
```
Wired in `App::new()` → `WriteGatePipeline::new(…, Box::new(TuiWriteGateObserver))`.

### `TuiAcpObserver`
Carries `tx` + `conv_id: String` (routes events to the correct conversation).
```
on_stream_chunk(text)        → Event::StreamChunk { conv_id, text }
on_tool_call_started(tool)   → Event::ToolCallStarted { conv_id, tool_name: tool }
on_streaming_status(status)  → Event::StreamingStatus { conv_id, status }
on_message_complete(role, c) → Event::MessageComplete { conv_id, role, content: c }
on_proposal_deferred(path,…) → Event::FileProposalDeferred { conv_id, path, additions, deletions }
```
One instance created per agent turn in `handle_send_message()`.

### `TuiSwarmObserver`
Carries `tx` only. Created once per swarm task.
```
on_phase_changed(p)             → Event::SwarmPhaseChanged(p)
on_agent_state_changed(id,s,d)  → Event::SwarmAgentStateChanged { id, status: s, detail: d }
on_tier_started(cur, tot)       → Event::SwarmTierStarted { current: cur, total: tot }
on_merge_conflict(b, files)     → Event::SwarmMergeConflict { branch: b, files }
on_completed(result)            → Event::SwarmCompleted(Box::new(result))
on_coordination_complete(dag)   → Event::SwarmCoordinationComplete { unit_count, summary }
on_tier_dispatch(id, tier, be)  → Event::SwarmTierDispatch { unit_id: id, tier, backend: be }
on_cost_update(est)             → Event::SwarmCostUpdate(est)
```

---

## Panel system

No panel trait. Panels are concrete structs; the render dispatch is a `match` in `App::render()`. Keeps rendering logic co-located with state, avoids dynamic dispatch.

```rust
match self.left_panel {
    LeftPanelMode::FileTree  => self.file_tree.render(area, buf, focused),
    LeftPanelMode::Search    => self.search_panel.render(area, buf, focused),
    LeftPanelMode::Review    => render_review_file_list(self, area, buf),
    LeftPanelMode::Changes   => render_changes_panel(self, area, buf),
}
match self.side_panel {
    SidePanelMode::AgentChat      => self.chat_state.render(area, buf, focused, &theme),
    SidePanelMode::SwarmDashboard => self.swarm_dashboard.render(area, buf, focused),
    SidePanelMode::GitPanel       => self.git_panel.render(area, buf, focused, &theme),
}
```

### Left panel modes

| Mode | State type | Key feature |
|---|---|---|
| `FileTree` | `FileTreeState` | Lazy children, single-child compaction, dialogs (new/rename/delete) |
| `Search` | `SearchPanelState` | Debounced live workspace search; `Enter` jumps to file:line |
| `Review` | `BatchReviewState` | Multi-file proposal list; `+N/-N` summaries; hunk-by-hunk acceptance |
| `Changes` | `ChangesState` | Git working-tree diff; M/A/D/R markers; click to view diff |

### Side panel modes

| Mode | State type | Key feature |
|---|---|---|
| `AgentChat` | `AgentChatState` | Multi-tab conversations, streaming, slash commands, `@file` autocomplete |
| `SwarmDashboard` | `SwarmDashboardState` | Agent table, per-agent activity log, diff overlay on completion |
| `GitPanel` | `GitPanelState` | Stage/unstage, commit, branch picker, amend |

---

## Editor buffer model

```rust
struct Buffer {
    text:         Rope,              // ropey — O(log n) insert/delete
    cursor:       Cursor,            // { line, col, anchor (selection start) }
    scroll:       Scroll,            // { top_line, left_col }
    undo_stack:   Vec<Transaction>,  // { range, replacement }
    redo_stack:   Vec<Transaction>,
    parser:       Option<Parser>,    // tree-sitter (incremental update on every edit)
    tree:         Option<Tree>,
    language:     Option<Language>,
    path:         Option<PathBuf>,   // None = scratch
    modified:     bool,
}
```

Syntax highlighting: `tree.root_node()` highlight queries run on visible line range only (never the whole file). Query results are cached per render frame.

---

## Render pipeline

```
App::render(frame)
  1. Layout computation
       vertical: tab_bar (1r) | main_area | status_bar (1r)
       horizontal: left_panel | editor | side_panel  (configurable ratios)
       terminal split appended below if visible
       all Rects cached in self.layout for mouse hit-tests

  2. Tab bar

  3. Left panel  (match self.left_panel → concrete render fn)

  4. Editor
       EditorView::render()
         syntax highlights (tree-sitter, visible range only)
         gutter (line numbers)
         styled text lines
         hunk overlay (if diff_review active)
         cursor cell
         scrollbar

  5. Markdown preview (if preview_visible && .md buffer)

  6. Side panel  (match self.side_panel → concrete render fn)

  7. Terminal  (vt100 cell grid → ratatui style map)

  8. Status bar  (panels/status_bar.rs)

  9. Set cursor position
```

---

## Layout

```
┌──────────────────────────────────────────────────┐
│                    Tab Bar                        │
├────────┬──────────────────────────┬───────────────┤
│        │                          │               │
│  Left  │        Editor            │  Side Panel   │
│ Panel  │     (center, largest)    │ (Agent Chat / │
│        │                          │  Swarm Dash / │
│        │                          │  Git Panel)   │
│        │                          │               │
├────────┴──────────────────────────┴───────────────┤
│                    Terminal                        │
├───────────────────────────────────────────────────┤
│                   Status Bar                      │
└───────────────────────────────────────────────────┘
```

### Focus model

`Focus` enum: `Editor | FileTree | SidePanel | Terminal`. Panel focus switched via Alt+Number (Alt+1 through Alt+4). If the target panel is hidden, it becomes visible and receives focus.

Keybinding layering: **Ctrl = editor/text** (word movement, selection, save, find, undo), **Alt = workspace** (panel focus, panel modes, tab cycling, line movement), **Shift = selection extension**, **F-keys = global toggles**.

### Layout presets

Six presets (Alt+Shift+1 through Alt+Shift+6) configure column widths:

| Preset | File Tree | Editor | Side Panel |
|---|---|---|---|
| 1 (default) | 15% | 60% | 25% |
| 2 (editor only) | 0% | 100% | 0% |
| ... | varying proportions | | |

---

## Shared widgets

### `ScrollState` (`widgets/scroll_state.rs`)

Scroll offset + single-item selection with cached viewport:
- `move_up()`, `move_down(item_count)` — selection with auto-scroll
- `page_up()`, `page_down(item_count)` — page-size jumps
- `scroll_up(n)`, `scroll_down(n, item_count)` — viewport-only scroll (mouse wheel)
- `ensure_visible()` — clamp scroll so selected item is in viewport
- `visible_range(item_count, viewport)` — iterator range for rendering
- Used by: `file_tree`, `search`, `swarm_dashboard`

### `TextInput` (`widgets/text_input.rs`)

Char-indexed text buffer with selection, undo/redo, word movement:
- `insert_char()`, `insert_str()`, `backspace()`, `delete()` — editing
- `move_left/right/home/end()`, `move_word_left/right()` — cursor movement
- `select_left/right()`, `select_word_left/right()`, `select_all()` — selection
- `undo()`, `redo()` — 50-entry undo stack
- Used by: `agent_chat`, `git_panel`, `search`, `app` (find bar)

---

## Async task patterns

### Agent chat task

```rust
tokio::spawn(async move {
    write_gate.lock().await.set_mode(WriteMode::Deferred);
    // run ACP pipeline → proposals accumulate in Deferred mode
    let proposals = write_gate.lock().await.take_pending_proposals();
    write_gate.lock().await.set_mode(WriteMode::Interactive);
    tx.send(Event::AcpTaskCompleted { conv_id, proposals });
});
```

`AcpTaskCompleted` triggers `App::enter_batch_review()` → left panel switches to `Review` mode.

### Swarm task (`/cswarm`)

```rust
tokio::spawn(async move {
    match pipeline::plan_coordinated(task, &config, coord_cfg, memory, &obs, make_obs).await {
        Ok(dsl_text) => {
            fs::write(&plan_path, dsl_text)?;
            tx.send(Event::SwarmDslPlanReady(plan_path));
        }
    }
});
```

`SwarmDslPlanReady` triggers `App::open_file(path)` → generated `.gaviero` file opens in editor for review before `/run`.

---

## Search panel (`panels/search.rs`)

```
┌─────────────────────┐
│ > query text|        │  ← TextInput with cursor (row 0)
│ 42 results           │  ← Summary (row 1)
│ src/foo.rs:12 match  │  ← Scrollable results (row 2+)
│ src/bar.rs:7  match  │
└─────────────────────┘
```

Two focus modes controlled by `editing: bool`:
- **Input mode** (`editing=true`): keystrokes update the query, search runs on every keystroke. Down/Enter moves to results.
- **Results mode** (`editing=false`): Up/Down navigate results. Enter opens file at matching line. Typing switches back.

### Editor find bar

`Ctrl+F` opens an inline find bar at the editor top. Search-as-you-type with match count indicator. `Enter`/`Down` → next match, `Up` → previous, `Esc` closes. `find_next_match()`/`find_prev_match()` on `Buffer` with wrap-around.

---

## Theme constants (`theme.rs`)

Centralised colour palette (~80 constants). All panels reference `theme::*` — no inline colour literals.

Key groups: `PANEL_BG`, `FOCUS_BORDER`, `TEXT_FG/DIM/BRIGHT`, `ACCENT`, `WARNING/SUCCESS/ERROR`, `DIFF_ADDED_BG/REMOVED_BG`, `TIER_CHEAP/EXPENSIVE`, `ACTIVITY_TOOL_CALL/STATUS`.

Constants: `CROSSTERM_POLL_MS = 50`, `TICK_INTERVAL_MS = 33` (30 fps), `TERMINAL_RESIZE_STEP = 5%`, `DIFF_PAGE_SCROLL = 10`.

---

## Concurrency model

- **Single main thread** for all state mutations and rendering
- **Background tasks** communicate exclusively via `mpsc::unbounded_channel<Event>`
- **No `Arc<Mutex<App>>`** — god-object pattern avoids lock contention
- **Write gate** is `Arc<Mutex<WriteGatePipeline>>` — the only shared mutable state accessed from both main thread and agent tasks
- Lock discipline: never hold write gate mutex across I/O or tree-sitter parsing

---

## Integration with gaviero-core

| Import | Used for |
|---|---|
| `Workspace` | Settings load, namespace resolution |
| `WriteGatePipeline`, `WriteMode` | Proposal staging, hunk review |
| `AcpPipeline` | Single-agent chat execution |
| `AcpSessionFactory` | Session lifecycle management |
| `swarm::pipeline::{execute, plan_coordinated}` | Multi-agent orchestration |
| `MemoryStore`, `memory::init` | Semantic memory |
| `GitRepo` | Git operations panel |
| `TerminalManager`, `TerminalEvent` | Embedded PTY |
| `SessionState` | Persist tab/layout state between runs |
| `tree_sitter::language_for_extension` | Syntax detection on file open |
| `observer::{WriteGateObserver, AcpObserver, SwarmObserver}` | Trait implementations |

---

## Design decisions

1. **God object for App.** Single struct avoids `Arc<Mutex<>>` lifetime issues; all state visible in one place; mutation is synchronous on the main thread.
2. **No panel trait.** Concrete structs + `match` dispatch keeps render logic co-located, avoids vtable indirection.
3. **Deferred write mode during streaming.** Proposals held until agent turn completes; prevents diff overlay flicker; enables batch review.
4. **Observer fire-and-forget.** Observer methods send into channel and return immediately. `ProposalCreated` boxes the proposal to avoid locking the write gate from the observer.
5. **`SwarmDslPlanReady` auto-opens file.** Generated `.gaviero` file is immediately visible in the editor; user can edit before running.
6. **Terminal focus pass-through.** When terminal is focused, raw key bytes go to PTY; only configured escape keys trigger TUI actions.
7. **Event drain up to 64.** Prevents render-bound bottleneck during high-frequency streaming without unbounded latency.

---

## Error handling strategy

- TUI wraps all fallible operations in `anyhow::Result`
- Errors in background tasks are sent as `Event` variants and displayed in status bar or chat panel
- Panic handler in `main.rs` restores terminal state before printing the backtrace
- Memory init failure: logs warning, continues with `memory = None`
- File watcher failure: logs warning, continues without live reload
