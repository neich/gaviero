# gaviero-tui — Architecture

`gaviero-tui` is the full-screen terminal UI binary. It holds no domain logic — all agent execution, swarm orchestration, write gating, memory, and git operations live in `gaviero-core`. The TUI's job is to render state, dispatch user input, and relay core events to UI updates via an mpsc channel.

---

## Module map

| Module | Purpose |
|---|---|
| `main` | Entry point: terminal init, EventLoop setup, async runtime, main render/event loop |
| `app` | `App` struct (~6 200 lines): all UI state, `handle_event()`, `render()`, command handlers |
| `event` | `Event` enum (43 variants) and `EventLoop` (channel + background task spawners) |
| `keymap` | Maps `crossterm::KeyEvent` → `Action`; handles chord prefixes (e.g. `[h`, `]h`) |
| `theme` | Color palette constants, tick/poll intervals, layout defaults |
| `editor/` | Text buffer, view renderer, syntax highlighting, diff overlay |
| `panels/` | Individual panel state structs and `render()` functions |
| `widgets/` | Reusable UI components (tab bar, text input, scrollbar) |

### `editor/` submodules

| File | Purpose |
|---|---|
| `buffer.rs` | `Buffer` struct: text rope, cursor, undo/redo, syntax tree |
| `view.rs` | `EditorView::render()`: gutter, syntax highlights, cursor, scrollbar |
| `highlight.rs` | Tree-sitter highlight queries → `StyledSpan` list per line |
| `markdown.rs` | Markdown preview renderer |
| `diff_overlay.rs` | Diff hunk visualiser for batch-review file list |

### `panels/` submodules

| File | Purpose |
|---|---|
| `agent_chat.rs` | `AgentChatState`: conversation history, streaming, input, attachments |
| `file_tree.rs` | `FileTreeState`: lazy-loaded directory tree, dialogs |
| `search.rs` | `SearchPanelState`: live workspace-wide search |
| `swarm_dashboard.rs` | `SwarmDashboardState`: agent tiles, tier progress, plan status |
| `git_panel.rs` | `GitPanelState`: staged/unstaged files, branch picker, commit input |
| `terminal.rs` | Terminal panel rendering helpers (screen cells, selection) |
| `status_bar.rs` | Context-sensitive bottom status line |
| `chat_markdown.rs` | Markdown rendering for chat messages (thinking blocks, code fences) |

### `widgets/` submodules

| File | Purpose |
|---|---|
| `tabs.rs` | File tab bar |
| `text_input.rs` | Single-line text input with cursor |
| `scrollbar.rs` | Vertical scrollbar widget |
| `scroll_state.rs` | Scroll position tracking helper |

---

## `App` struct — state field groups

```rust
pub struct App {
    // ── Workspace & buffers ──────────────────────────────────────────
    workspace:          Workspace,
    buffers:            Vec<Buffer>,          // open tabs
    active_buffer:      usize,

    // ── Focus & layout ───────────────────────────────────────────────
    focus:              Focus,                // Editor | FileTree | SidePanel | Terminal
    left_panel:         LeftPanelMode,        // FileTree | Search | Review | Changes
    side_panel:         SidePanelMode,        // AgentChat | SwarmDashboard | GitPanel
    panel_visible:      PanelVisibility,      // { file_tree, side_panel, terminal }
    layout_presets:     Vec<LayoutPreset>,    // Alt+5–Alt+0
    layout:             LayoutAreas,          // cached Rect per panel (for mouse hit-test)

    // ── Left panel state ─────────────────────────────────────────────
    file_tree:          FileTreeState,
    search_panel:       SearchPanelState,
    changes_state:      Option<ChangesState>,

    // ── Write gate & review ──────────────────────────────────────────
    write_gate:         Arc<Mutex<WriteGatePipeline>>,
    diff_review:        Option<DiffReviewState>,    // single-file hunk overlay
    batch_review:       Option<BatchReviewState>,   // multi-file proposal list

    // ── Chat & agents ────────────────────────────────────────────────
    chat_state:         AgentChatState,
    acp_tasks:          HashMap<String, JoinHandle<()>>,
    memory:             Option<Arc<MemoryStore>>,

    // ── Swarm ────────────────────────────────────────────────────────
    swarm_dashboard:    SwarmDashboardState,

    // ── Terminal ─────────────────────────────────────────────────────
    terminal_manager:   TerminalManager,
    terminal_selection: TerminalSelectionState,

    // ── Git ──────────────────────────────────────────────────────────
    git_repo:           Option<GitRepo>,
    git_panel:          GitPanelState,

    // ── Editor helpers ───────────────────────────────────────────────
    find_bar_active:    bool,
    preview_visible:    bool,
    highlight_configs:  HashMap<String, HighlightConfig>,

    // ── UI chrome ────────────────────────────────────────────────────
    theme:              Theme,
    status_message:     Option<(String, Instant)>, // transient toast (~3 s)
    should_quit:        bool,

    // ── Mouse ────────────────────────────────────────────────────────
    mouse_dragging:     bool,
    scrollbar_dragging: Option<ScrollbarTarget>,
    tree_dialog:        Option<TreeDialog>,
}
```

---

## Event loop

```
┌────────────────────────┐
│  crossterm (key/mouse) │──┐
│  file watcher (notify) │──┤
│  tick timer (30 fps)   │──┼──→  mpsc::unbounded_channel  ──→  Event
│  terminal bridge (PTY) │──┘                                      │
└────────────────────────┘                                         ▼
                                                        ┌──────────────────────┐
                                                        │  main loop (main.rs) │
                                                        │                      │
                                                        │  terminal.draw(      │
                                                        │    app.render(frame) │◄──────────┐
                                                        │  )                   │           │
                                                        │                      │     state │
                                                        │  event = rx.recv()   │  mutation │
                                                        │  app.handle_event()  │───────────┘
                                                        │                      │
                                                        │  drain up to 64      │
                                                        │  pending events      │
                                                        └──────────────────────┘
```

**Drain strategy:** After processing the first event, the loop calls `rx.try_recv()` up to 64 more times before redrawing. This prevents frame-rate bottlenecks during high-frequency bursts (e.g. agent streaming 50+ `StreamChunk` events/second).

### Event producers (background tasks)

| Producer | Spawned in | Events sent |
|---|---|---|
| `spawn_crossterm_reader()` | `EventLoop` | `Key`, `Mouse`, `Resize`, `Paste` |
| `spawn_file_watcher()` | `EventLoop` | `FileChanged`, `FileTreeChanged` |
| `spawn_tick_timer()` | `EventLoop` | `Tick` (30 fps) |
| `spawn_terminal_bridge()` | `EventLoop` | `Terminal(TerminalEvent)` |
| Agent `tokio::spawn` tasks | `handle_send_message` | `StreamChunk`, `ToolCallStarted`, `MessageComplete`, `FileProposalDeferred`, `AcpTaskCompleted` |
| Swarm `tokio::spawn` tasks | `handle_swarm_command` | `SwarmPhaseChanged`, `SwarmAgentStateChanged`, `SwarmTierStarted`, `SwarmCompleted`, `SwarmCoordinationStarted`, `SwarmCoordinationComplete`, `SwarmTierDispatch`, `SwarmDslPlanReady` |
| Memory init task | `main.rs` | `MemoryReady` |

---

## Observer implementations

Three types implement the core observer traits. All methods are non-blocking: they call `tx.send(Event::*)` and return immediately.

### `TuiWriteGateObserver`

```rust
on_proposal_created(proposal) → tx.send(Event::ProposalCreated(Box::new(proposal)))
on_proposal_updated(id)       → tx.send(Event::ProposalUpdated(id))
on_proposal_finalized(path)   → tx.send(Event::ProposalFinalized(path))
```

Wired in `App::new()`, passed to `WriteGatePipeline::new()`.

### `TuiAcpObserver`

Carries `tx: Sender` and `conv_id: String` (conversation identifier).

```rust
on_stream_chunk(_, text)           → tx.send(Event::StreamChunk { conv_id, text })
on_tool_call_started(_, tool)      → tx.send(Event::ToolCallStarted { conv_id, tool_name: tool })
on_streaming_status(_, status)     → tx.send(Event::StreamingStatus { conv_id, status })
on_message_complete(_, role, body) → tx.send(Event::MessageComplete { conv_id, role, content: body })
on_proposal_deferred(_, path, +,-) → tx.send(Event::FileProposalDeferred { conv_id, path, additions, deletions })
```

Created per agent turn in `handle_send_message()`; `conv_id` routes events to the right chat conversation.

### `TuiSwarmObserver`

Carries only `tx: Sender`. Used for both `/swarm` and `/cswarm`.

```rust
on_phase_changed(phase)                → tx.send(Event::SwarmPhaseChanged(phase))
on_agent_state_changed(id, st, detail) → tx.send(Event::SwarmAgentStateChanged { id, status: st, detail })
on_tier_started(cur, tot)              → tx.send(Event::SwarmTierStarted { current: cur, total: tot })
on_merge_conflict(branch, files)       → tx.send(Event::SwarmMergeConflict { branch, files })
on_completed(result)                   → tx.send(Event::SwarmCompleted(Box::new(result)))
on_coordination_started(prompt)        → tx.send(Event::SwarmCoordinationStarted(prompt))
on_coordination_complete(n, summary)   → tx.send(Event::SwarmCoordinationComplete { unit_count: n, summary })
on_tier_dispatch(id, tier, backend)    → tx.send(Event::SwarmTierDispatch { unit_id: id, tier, backend })
on_cost_update(est)                    → tx.send(Event::SwarmCostUpdate(est))
```

---

## Editor buffer model

```rust
pub struct Buffer {
    text:              Rope,                      // ropey — O(log n) insert/delete
    cursor:            Cursor,                    // { line, col, anchor (selection start) }
    scroll:            Scroll,                    // { top_line, left_col }
    undo_stack:        Vec<Transaction>,
    redo_stack:        Vec<Transaction>,
    parser:            Option<Parser>,            // tree-sitter parser instance
    tree:              Option<Tree>,              // cached syntax tree
    language:          Option<Language>,          // tree-sitter language
    lang_name:         Option<String>,            // "rust", "python", …
    path:              Option<PathBuf>,           // None = scratch buffer
    modified:          bool,
    indent_query:      Option<Arc<Query>>,
    format_level:      FormatLevel,               // Compact | Normal | Expanded
    search_highlight:  Option<String>,
    search_matches:    Vec<(line, start_col, end_col)>,
}
```

**Tab model:** `app.buffers` is a `Vec<Buffer>`; `app.active_buffer` is the index. `Ctrl+T` pushes a new empty `Buffer`; `Ctrl+W` removes the current one.

**Undo/redo:** Each mutating operation appends a `Transaction` (list of `Change { range, replacement }`) to `undo_stack` and clears `redo_stack`. `Ctrl+Z` pops from `undo_stack`, reverses changes, pushes to `redo_stack`. `Ctrl+Y` is the inverse.

**Syntax tree:** The tree-sitter `Tree` is updated incrementally on every edit via `parser.parse(new_text, Some(old_tree))`. Highlight queries run over only the visible line range during render.

---

## Panel architecture

Panels are **not a trait hierarchy**. Each panel is a concrete struct with a `render()` function. `App::render()` dispatches to panel render methods via `match` arms:

```rust
// In App::render():
match self.left_panel {
    LeftPanelMode::FileTree  => self.file_tree.render(area, buf, focused),
    LeftPanelMode::Search    => self.search_panel.render(area, buf, focused),
    LeftPanelMode::Review    => render_review_file_list(self, area, buf),
    LeftPanelMode::Changes   => render_changes_panel(self, area, buf),
}

match self.side_panel {
    SidePanelMode::AgentChat       => self.chat_state.render(area, buf, focused, &self.theme),
    SidePanelMode::SwarmDashboard  => self.swarm_dashboard.render(area, buf, focused),
    SidePanelMode::GitPanel        => self.git_panel.render(area, buf, focused, &self.theme),
}
```

This avoids dynamic dispatch overhead and keeps all rendering logic co-located with state.

---

## Render pipeline

```
App::render(frame: &mut Frame)
  │
  ├─ 1. Layout computation
  │       split vertically: [ tab_bar | main_area | status_bar ]
  │       if terminal visible: split main_area → [ panels_area | terminal_area ]
  │       split panels_area horizontally: [ file_tree% | editor% | side_panel% ]
  │       cache all Rects in self.layout (for mouse hit-testing)
  │
  ├─ 2. Tab bar  (widgets/tabs.rs)
  │
  ├─ 3. Left panel  (match self.left_panel)
  │
  ├─ 4. Editor  →  EditorView::render(area, buf)
  │       ├─ compute syntax highlights for visible lines
  │       ├─ render gutter (line numbers, scaled width)
  │       ├─ render lines with StyledSpans
  │       ├─ if diff_review active: render hunk overlay
  │       ├─ render cursor cell (inverted)
  │       └─ render scrollbar
  │
  ├─ 5. Markdown preview  (if preview_visible && .md buffer)
  │       split editor area → [ code | preview ]
  │
  ├─ 6. Side panel  (match self.side_panel)
  │
  ├─ 7. Terminal  (if terminal_area.is_some())
  │       render_terminal_screen(area, buf, vt100_screen, selection, theme)
  │
  ├─ 8. Status bar  (panels/status_bar.rs)
  │       context-sensitive content based on focus + mode
  │
  └─ 9. Cursor position update
          frame.set_cursor(editor_cursor_screen_pos)
```

---

## Async task pattern

Agent runs and swarm runs execute in `tokio::spawn` tasks. They **never mutate `App` directly** — all state updates flow through the event channel.

### Agent task pattern (chat)

```rust
tokio::spawn(async move {
    // 1. Set WriteGate to Deferred — accumulate proposals without showing them yet
    write_gate.lock().await.set_mode(WriteMode::Deferred);

    // 2. Build enriched prompt (conversation history + file attachments + memory)
    let enriched = enrich_prompt(prompt, memory, history).await;

    // 3. Run agent
    let obs = TuiAcpObserver { tx: tx.clone(), conv_id };
    AcpPipeline::new(obs, write_gate.clone())
        .send_prompt(enriched, …)
        .await;

    // 4. Collect deferred proposals
    let proposals = write_gate.lock().await.take_pending_proposals();
    write_gate.lock().await.set_mode(WriteMode::Interactive);

    // 5. Notify main loop
    if proposals.is_empty() {
        // MessageComplete was already sent by AcpObserver
    } else {
        tx.send(Event::AcpTaskCompleted { conv_id, proposals });
    }
});
```

`Event::AcpTaskCompleted` triggers `App::enter_batch_review()` — the left panel switches to Review mode.

### Swarm task pattern (`/cswarm`)

```rust
tokio::spawn(async move {
    let obs = TuiSwarmObserver { tx: tx.clone() };
    match pipeline::plan_coordinated(task, &config, coord_cfg, memory, &obs, make_obs).await {
        Ok(dsl_text) => {
            // Write to tmp/gaviero_plan_<ts>.gaviero
            fs::write(&plan_path, dsl_text);
            tx.send(Event::SwarmDslPlanReady(plan_path));
        }
        Err(e) => {
            tx.send(Event::SwarmPhaseChanged("failed".into()));
            tx.send(Event::MessageComplete { content: e.to_string(), … });
        }
    }
});
```

`Event::SwarmDslPlanReady` triggers `App::open_file(path)` + system chat message.

---

## Integration

### Imports from `gaviero-core`

| Symbol | Used for |
|---|---|
| `Workspace` | Settings and namespace resolution |
| `WriteGatePipeline`, `WriteMode` | Proposal staging and hunk review |
| `AcpPipeline` | Single-agent chat |
| `swarm::pipeline::{execute, plan_coordinated}` | Multi-agent orchestration |
| `swarm::pipeline::SwarmConfig` | Swarm configuration |
| `MemoryStore`, `memory::init` | Semantic memory |
| `GitRepo` | Git panel |
| `WorktreeManager` | Worktree provisioning |
| `TerminalManager`, `TerminalEvent` | Embedded PTY terminal |
| `session_state::SessionState` | Persisted tab/layout state |
| `tree_sitter::language_for_extension` | Buffer syntax detection |
| `observer::{WriteGateObserver, AcpObserver, SwarmObserver}` | Trait implementations |

### Imports from `gaviero-dsl`

| Symbol | Used for |
|---|---|
| `compile(source, filename, None, None)` | Execute `/run <file.gaviero>` command |

### Key dependencies

| Crate | Purpose |
|---|---|
| `ratatui 0.30` | Terminal UI rendering (widgets, layout, frame) |
| `crossterm 0.29` | Raw mode, keyboard/mouse events, ANSI sequences |
| `ropey 1.6` | Text rope for efficient buffer editing |
| `tokio` | Async runtime, `spawn`, channels |
| `notify 7` | File system watcher (reload on external edit) |
| `arboard 3` | System clipboard (copy/paste) |
| `vt100` / `tui-term` | VT100 screen state for terminal rendering |
| `unicode-width` | Grapheme cluster width for cursor positioning |
| `serde` / `toml` | Config and session state serialisation |

---

## Design decisions

1. **App as God Object, deliberately.** All state lives in one struct. This avoids lifetime/borrow-checker fights that arise when splitting state across multiple Arc<Mutex<>> holders that need to reference each other during event handling.

2. **No panel trait.** Panels are concrete structs dispatched via `match` in `App::render()`. This keeps all rendering logic co-located, avoids dynamic dispatch, and makes it easy to grep for "who renders what".

3. **Drain up to 64 events per frame.** The main loop processes pending events before redrawing. This prevents the render rate from becoming the bottleneck during streaming bursts without unbounded latency accumulation.

4. **Deferred write mode during streaming.** Proposals accumulated during an agent turn are held back until the turn completes. This prevents the diff overlay from appearing and disappearing mid-stream, and enables batch review of all changes at once.

5. **Observer sends events, never data.** Observer methods fire-and-forget into the channel. The `ProposalCreated` event boxes the full proposal so the main thread never needs to lock the write gate to display it.

6. **Cached layout rectangles.** `self.layout` is recomputed at the start of every `render()` call. The cached `Rect` values are used by `handle_mouse()` for hit-testing without a second layout pass.

7. **`SwarmDslPlanReady` opens file in editor.** When the coordinator finishes, the generated `.gaviero` file is opened in the editor automatically. The user can edit the plan before running it, which is the primary mechanism for correcting phantom file references.
