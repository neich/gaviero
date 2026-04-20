# gaviero-tui — Architecture

Full-screen terminal editor. Rendering + event routing only. All runtime logic delegates to `gaviero-core`.

---

## 1. Module Layout

```
gaviero-tui/src/
├─ main.rs                 Terminal setup, panic handler, event loop
├─ app.rs                  App struct (state + layout + focus)
├─ event.rs                Event enum (43+ variants), source plumbing
├─ keymap.rs               Action enum, keybindings, chord system
├─ theme.rs                One Dark palette, timing constants
├─ editor/
│  ├─ buffer.rs            Ropey-backed buffer, cursor, selection, undo/redo
│  ├─ view.rs              Viewport, gutter, syntax cache, cursor
│  ├─ diff_overlay.rs      DiffReviewState, hunk accept/reject, navigation
│  ├─ highlight.rs         Tree-sitter query runner → StyledSpan
│  └─ markdown.rs          In-editor markdown rendering
├─ panels/
│  ├─ file_tree.rs         Multi-root tree, git decorations, proposal badges
│  ├─ agent_chat.rs        AgentChatState, history, @file autocomplete
│  ├─ chat_markdown.rs     ChatLine rendering
│  ├─ swarm_dashboard.rs   Agent table, tier/phase indicators, cost
│  ├─ git_panel.rs         Staging, commit, branch picker
│  ├─ terminal.rs          Embedded PTY render (tui-term)
│  ├─ status_bar.rs        Mode / file / branch / agent status
│  └─ search.rs            SearchPanelState, live results
├─ widgets/
│  ├─ tabs.rs              TabBar
│  ├─ scrollbar.rs         Custom scrollbar
│  ├─ scroll_state.rs      Viewport-aware offset + selection
│  ├─ text_input.rs        Line input with cursor/selection/undo
│  └─ render_utils.rs      Shared helpers
└─ app/
   ├─ controller.rs        Top-level event handling + action dispatch
   ├─ layout.rs            5-area layout computation
   ├─ render.rs            Draw orchestration
   ├─ left_panel.rs        Left panel modes (tree / search / changes / review)
   ├─ review.rs            Diff acceptance flows
   ├─ side_panel.rs        Side panel modes (chat / swarm / git)
   ├─ commands.rs          Slash commands (/run, /swarm, /cswarm, /remember, …)
   ├─ editing.rs           Editor + find-bar interactions
   ├─ session.rs           Session restore/save (session_state bridge)
   ├─ state.rs             Shared enums/structs
   └─ observers.rs         WriteGateObserver / AcpObserver / SwarmObserver
                           impls bridging to Event
```

---

## 2. Core Abstractions

### `App`

Owns tabs, panel states, focus, theme, and event channel sender. The main loop calls `app.render(frame)` then `app.handle_event(event)` exclusively from one task.

### Focus

```rust
pub enum Focus { Editor, FileTree, SidePanel, Terminal }
```

`Alt+1..4` switch focus; Fullscreen hides the non-focused areas.

### Panel modes

```rust
pub enum LeftPanelMode  { FileTree(..), Search(..), Changes(..), Review(..) }
pub enum SidePanelMode  { AgentChat(..), SwarmDashboard(..), GitPanel(..) }
```

Proposals auto-switch the left panel to `Review`; the user navigates hunks with `]h`/`[h`, accepts/rejects with `a`/`r` (or `A`/`R` for all), finalizes with `f`, aborts with `q`.

### Observer bridges (`app/observers.rs`)

Three trait implementations hold an `mpsc::UnboundedSender<Event>` and translate core callbacks into `Event` variants.

---

## 3. Event Loop

```
            ┌───────── crossterm reader (dedicated thread)
            ├───────── notify file watcher (tokio task)
            ├───────── tick timer (tokio::time::interval 33ms)
            ├───────── terminal bridge (PTY thread)
            ├───────── WriteGateObserver impl
            ├───────── AcpObserver impl
            └───────── SwarmObserver impl
                           │
                           ▼
              mpsc::unbounded_channel<Event>
                           │
                           ▼
           loop {
               terminal.draw(|f| app.render(f));
               event = event_rx.recv().await;
               app.handle_event(event);
               for _ in 0..64 { try_recv → handle_event }   // drain
               if app.should_quit { break; }
           }
```

Golden rule: **no background task mutates `App` directly**. External sources push events; the main loop is the sole mutator.

---

## 4. Layout (5 areas)

```
┌─────────────── Tab Bar ─────────────────────────┐
├──────┬───────────────────────┬──────────────────┤
│ Left │      Editor           │   Side Panel     │
│Panel │  (ropey + view)       │ chat / swarm /   │
│      │                       │ git              │
├──────┴───────────────────────┴──────────────────┤
│              Terminal (embedded PTY)            │
├─────────────────────────────────────────────────┤
│                Status Bar                       │
└─────────────────────────────────────────────────┘
```

Computed by `app/layout.rs::compute_layout(full_area, …)`.

---

## 5. Editor

- **Buffer** (`editor/buffer.rs`): `ropey::Rope`, cursor `(line, col)`, selection, undo/redo transactions, find forward/backward.
- **View** (`editor/view.rs`): viewport rect, scroll offsets, per-line syntax cache keyed by line hash, only the visible range is highlighted.
- **Highlight** (`editor/highlight.rs`): tree-sitter queries loaded from `queries/{lang}/highlights.scm` via `gaviero_core::query_loader`. Results are styled spans consumed by `EditorView::render`.
- **Diff overlay** (`editor/diff_overlay.rs`): visualizes `StructuralHunk`s side-by-side; `accept_hunk(id, i)` and `accept_node(id, name)` delegate to `write_gate`.

---

## 6. Commands (slash commands)

| Command | Handler | Effect |
|---|---|---|
| `/run <file.gaviero> [prompt]` | `commands.rs::handle_run` | `gaviero_dsl::compile` then `swarm::pipeline::execute` |
| `/swarm <text>` | `handle_swarm` | Natural-language swarm planning + execute |
| `/cswarm <text>` | `handle_cswarm` | Coordinator-only: emit `.gaviero` plan file |
| `/remember <text>` | `handle_remember` | `MemoryStore::store_scoped` |
| `/attach <path>` / `/detach <path>` | chat attachments | |
| `/set-model <spec>` | override next message model | |
| `/clear` | clear conversation | |

All handlers run async; results arrive through the observer event channel.

---

## 7. Session Persistence

`app/session.rs` bridges to `gaviero_core::session_state`: `SessionState` carries workspace path, open tabs (`TabState`), current tab, panel modes, terminal visibility, and `StoredConversation` history. `ConversationIndex` + `StoredConversation` JSON files live under `.gaviero/state/`. `load_session` on startup, `save_session` on timer and shutdown.

---

## 8. Concurrency

Single-threaded UI + async producers. No Mutex in TUI state. Observer `Arc`s are cloned into core tasks; events flow one-way into the main loop channel.

---

## 9. Error Handling

- User-facing failures appear in the status bar or a transient alert.
- Swarm / ACP errors arrive as `Event::SwarmCompleted(failed)` / `Event::AcpTaskCompleted(Err)` and render into the relevant side panel.
- Panic handler in `main.rs` restores the terminal before unwinding.

---

## 10. Dependencies

- `ratatui 0.30` + `crossterm 0.29`
- `ropey` — rope buffer
- `notify` — filesystem watch
- `portable-pty` + `vt100` + `tui-term` — embedded terminal
- `arboard` — clipboard
- `gaviero-core`, `gaviero-dsl`

---

## 11. API Surface

No public library API. Binary entry is `main.rs`; everything else is crate-private.

---

See [CLAUDE.md](CLAUDE.md) for build, test, and keybinding details.
