# gaviero-tui — Architecture

Full-screen terminal editor. Rendering + event routing only — all runtime logic delegates to [`gaviero-core`](../gaviero-core). Binary: `gaviero`.

---

## 1. Topology

```
        gaviero-core             gaviero-dsl
              ▲                       ▲
              │ observer traits       │ compile / compile_file
              │ + WriterHandle        │
              │                       │
        ┌─────┴───────────────────────┴─────┐
        │            gaviero-tui            │
        │   ┌─────────┐    ┌─────────────┐  │
        │   │  App    │◄───┤  Event      │  │
        │   │ state + │    │  loop       │  │
        │   │ render  │───►│  (single    │  │
        │   └─────────┘    │  mpsc)      │  │
        │        ▲          └─────┬──────┘  │
        │        │                │         │
        │   crossterm │ notify │ tick │ obs │
        └────────────┴────────┴──────┴──────┘
```

Single-threaded UI loop with async producers. Core observer impls live in [`app/observers.rs`](src/app/observers.rs); they translate every callback into an `Event` and push it onto one channel.

---

## 2. Module Layout

```
gaviero-tui/src/
├─ main.rs                 Terminal setup, panic handler, event loop.
│                          Dispatches on argv: folder → Workspace::single_folder,
│                          *.gaviero-workspace → Workspace::load.
├─ app.rs                  App struct (state + layout + focus)
├─ event.rs                Event enum (45+ variants), source plumbing
├─ keymap.rs               Action enum, keybindings, chord system
│                          (Alt+Z = ToggleWordWrap)
├─ theme.rs                One Dark palette, timing constants
├─ editor/
│  ├─ buffer.rs            Ropey-backed buffer, cursor, selection, undo/redo
│  ├─ view.rs              Viewport, gutter, syntax cache, cursor
│  ├─ wrap.rs              Visual-line layout for word-wrapped rendering
│  │                       (WrapLayout, VisualSegment, unicode-width-aware)
│  ├─ diff.rs              LCS line diff for diff-view buffers (DiffKind
│  │                       Context/Added/Removed), shared by Changes panel
│  │                       and the read-only diff overlay tab
│  ├─ diff_overlay.rs      DiffReviewState, hunk accept/reject, navigation
│  ├─ highlight.rs         Tree-sitter query runner → StyledSpan
│  └─ markdown.rs          In-editor markdown rendering
├─ panels/
│  ├─ file_tree.rs         Multi-root tree, git decorations, proposal badges
│  ├─ agent_chat.rs        AgentChatState, history, @file autocomplete,
│  │                       context-pressure indicator, bootstrap-tokens
│  │                       indicator, slash-command dispatch (/lite, /help,
│  │                       /compact, /context, /clear, …)
│  ├─ chat_markdown.rs     ChatLine rendering
│  ├─ swarm_dashboard.rs   Agent table, tier/phase indicators, cost
│  ├─ git_panel.rs         Staging, commit, branch picker
│  ├─ terminal.rs          Embedded PTY render (tui-term)
│  ├─ status_bar.rs        Mode / file / branch / agent status +
│  │                       word-wrap "│ Wrap" indicator
│  ├─ search.rs            SearchPanelState, live results
│  └─ memory_panel.rs      Memory inspection / management (Tier A4):
│                          Injected Now / Recently Written /
│                          Scope Summary / Search; destructive ops
│                          enqueue WriterMessage::PanelEdit
├─ widgets/
│  ├─ tabs.rs              TabBar
│  ├─ scrollbar.rs         Custom scrollbar
│  ├─ scroll_state.rs      Viewport-aware offset + selection
│  ├─ text_input.rs        Line input with cursor/selection/undo
│  └─ render_utils.rs      Shared helpers (incl. strip_ansi)
└─ app/
   ├─ controller.rs        Top-level event handling + action dispatch
   ├─ layout.rs            5-area layout computation
   ├─ render.rs            Draw orchestration; emits bootstrap-token
   │                       measurement + context-pressure events
   ├─ left_panel.rs        Left panel modes (tree / search / changes / review)
   ├─ review.rs            Diff acceptance flows
   ├─ side_panel.rs        Side panel modes (chat / swarm / git / memory)
   ├─ commands.rs          Slash commands (/run, /swarm, /cswarm,
   │                       /undo-swarm, /remember*, /forget*, /restore,
   │                       /reembed, /sleep, /consolidate-session,
   │                       /attach, /detach, …)
   ├─ editing.rs           Editor + find-bar interactions; receives
   │                       viewport width for word-wrapped cursor moves
   ├─ session.rs           Session restore/save (session_state bridge)
   │                       + per-folder topology cache (async builder
   │                       fronting repo_map::build_folder_topology)
   ├─ chat_memory.rs       build_turn_transcript +
   │                       consolidate_conversation: feeds the S3
   │                       extractor and the per-conversation
   │                       Consolidator through WriterHandle /
   │                       WriterMessage::TurnComplete
   ├─ state.rs             Shared enums/structs
   └─ observers.rs         WriteGateObserver / AcpObserver /
                          SwarmObserver / MemoryObserver /
                          ManifestObserver impls → Event
```

---

## 3. Core Abstractions

### `App` ([`src/app.rs`](src/app.rs))

Owns tabs, panel states, focus, theme, the workspace handle, optional `MemoryStores` + `WriterHandle`, the per-folder topology cache, and the event channel sender. The main loop calls `app.render(frame)` then `app.handle_event(event)` exclusively from one task — every state mutation goes through `handle_event`.

### Focus

```rust
pub enum Focus { Editor, FileTree, SidePanel, Terminal }
```

`Alt+1..4` switch focus; Fullscreen (`F11`) hides the non-focused areas.

### Panel modes

```rust
pub enum LeftPanelMode  { FileTree(..), Search(..), Changes(..), Review(..) }
pub enum SidePanelMode  { AgentChat(..), SwarmDashboard(..), GitPanel(..),
                          Memory(..) }
```

Proposals auto-switch the left panel to `Review`; the user navigates hunks with `]h`/`[h`, accepts/rejects with `a`/`r` (or `A`/`R` for all), finalizes with `f`, aborts with `q`.

### Observer bridges ([`src/app/observers.rs`](src/app/observers.rs))

The TUI implements [`WriteGateObserver`](../gaviero-core/src/observer.rs), [`AcpObserver`](../gaviero-core/src/observer.rs), [`SwarmObserver`](../gaviero-core/src/observer.rs), [`MemoryObserver`](../gaviero-core/src/memory/observer.rs), and `ManifestObserver`. Each holds an `mpsc::UnboundedSender<Event>` and translates core callbacks into `Event` variants — including `MemoryWriteCommitted`, `ManifestPersisted`, `BootstrapTokensMeasured`, and the new Cursor-specific `CursorSessionStarted(session_id)` event that flows into `SessionLedger::continuity_handle`.

### Workspace dispatch

[`main.rs`](src/main.rs) checks the argv extension at launch: a directory becomes `Workspace::single_folder(path)`, a `*.gaviero-workspace` file becomes `Workspace::load(path)`. Workspace files carry multiple folders that the agent session forwards as `--add-dir` flags.

### Topology cache ([`app/session.rs`](src/app/session.rs))

`get_or_build_topology_cached(...)` builds `repo_map::build_folder_topology` once per folder on a background tokio task and caches the result behind an `RwLock`. Used to inject `<repo_topology>` cheaply on first turns; the cache key is the folder root path.

---

## 4. Event Loop

```
            ┌───────── crossterm reader (dedicated thread)
            ├───────── notify file watcher (tokio task)
            ├───────── tick timer (tokio::time::interval 33ms)
            ├───────── terminal bridge (PTY thread)
            ├───────── WriteGateObserver impl
            ├───────── AcpObserver impl
            ├───────── SwarmObserver impl
            ├───────── MemoryObserver impl
            └───────── ManifestObserver impl
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

Golden rule: **no background task mutates `App` directly**. External sources push events; the main loop is the sole mutator. Render is pure.

---

## 5. Layout (5 areas)

```
┌─────────────── Tab Bar ─────────────────────────┐
├──────┬───────────────────────┬──────────────────┤
│ Left │      Editor           │   Side Panel     │
│Panel │  (ropey + view +      │ chat / swarm /   │
│      │   wrap + highlight)   │ git / memory     │
├──────┴───────────────────────┴──────────────────┤
│              Terminal (embedded PTY)            │
├─────────────────────────────────────────────────┤
│ Status Bar  mode | file | branch | wrap | agent │
└─────────────────────────────────────────────────┘
```

Computed by [`app/layout.rs::compute_layout`](src/app/layout.rs).

---

## 6. Editor

- **Buffer** ([`editor/buffer.rs`](src/editor/buffer.rs)): `ropey::Rope`, cursor `(line, col)`, selection, undo/redo transactions, find forward/backward, `word_wrap: bool` toggle.
- **Wrap layout** ([`editor/wrap.rs`](src/editor/wrap.rs)): `WrapLayout::build(&Buffer, content_width)` produces `Vec<VisualSegment>` (logical line slices by char column) honouring `unicode-width`. Visual-line lookup is `O(segments)`; used by `view.rs` and editing actions that need viewport-aware cursor motion. Disabled when `buffer.word_wrap == false` (each logical line maps to one segment).
- **View** ([`editor/view.rs`](src/editor/view.rs)): viewport rect, scroll offsets, per-line syntax cache keyed by line hash; only the visible range is highlighted. Consumes `WrapLayout` when word wrap is on.
- **Highlight** ([`editor/highlight.rs`](src/editor/highlight.rs)): tree-sitter queries loaded from `queries/{lang}/highlights.scm` via [`gaviero_core::query_loader`](../gaviero-core/src/query_loader.rs). Results are styled spans consumed by `EditorView::render`.
- **Diff overlay** ([`editor/diff_overlay.rs`](src/editor/diff_overlay.rs)): visualizes `StructuralHunk`s side-by-side; `accept_hunk(id, i)` and `accept_node(id, name)` delegate to [`write_gate`](../gaviero-core/src/write_gate.rs).
- **Line diff** ([`editor/diff.rs`](src/editor/diff.rs)): LCS-based line diff returning `Vec<(DiffKind, String)>`; backs the Changes panel and the read-only diff-view buffer.

---

## 7. Slash Commands

Dispatched in [`app/commands.rs`](src/app/commands.rs) and [`panels/agent_chat.rs`](src/panels/agent_chat.rs).

| Command | Handler | Effect |
|---|---|---|
| `/run <file.gaviero> [prompt]` | `commands.rs::handle_run_script_command` | [`gaviero_dsl::compile_file`](../gaviero-dsl/src/lib.rs) then [`swarm::pipeline::execute`](../gaviero-core/src/swarm/pipeline.rs) |
| `/swarm <text>` | `handle_swarm_command` | Coordinator plans + executes |
| `/cswarm <text>` | `handle_coordinated_swarm_command` | Coordinator-only: emit `.gaviero` plan file |
| `/undo-swarm` | `handle_undo_swarm_command` | Revert files from the last `/cswarm` run |
| `/lite` (alias `/minimal`) | `agent_chat.rs` | Per-turn arm: skip `<repo_outline>`, memory, impact; keep `<repo_topology>` |
| `/model <provider:model>` | `agent_chat.rs` | Per-conversation model override |
| `/effort <off..max>` | `agent_chat.rs` | Reasoning/effort level for Claude + Codex |
| `/compact [N]` | `agent_chat.rs` | Keep last N messages |
| `/context` | `agent_chat.rs` | Print context-pressure breakdown |
| `/clear` (alias `/reset`) | `agent_chat.rs` | Clear agent context (keeps visible history) |
| `/workspace` (alias `/ws`) | `agent_chat.rs` | Arm workspace-wide planner scope for next prompt |
| `/remember[-here\|-module\|-workspace\|-global] <text>` | `handle_remember_command` | `WriterHandle::send(Store…)` at the chosen scope |
| `/forget <…>` / `/forget-scope` / `/forget-type` / `/forget-source` | `handle_forget_command` | Soft-delete via `WriterMessage::Forget`; writes `deletions` audit row |
| `/forget-history [--confirm] <id>` | `handle_forget_history_command` | C2.4 in-place History redaction |
| `/restore <id>` (or `--since`) | `handle_restore_command` | Replay deletion through dedup |
| `/reembed` | `handle_reembed_command` | Re-embed every memory under the configured embedder |
| `/sleep [--dry-run]` | `handle_sleep_command` | Run sleeptime hygiene pass |
| `/consolidate-session` | `handle_consolidate_session_command` | End-of-session consolidator on the active conversation |
| `/attach <path>` / `/detach <name\|all>` | `handle_attach_command` / `handle_detach_command` | Chat attachments |
| `/help` | `agent_chat.rs` | List all commands |
| `//<cmd>` | `agent_chat.rs` | Forward `/<cmd>` verbatim to the agent (Claude Code skills, e.g. `//init`) |

All handlers run async; results arrive through the observer event channel.

---

## 8. Chat ↔ Memory Bridge ([`app/chat_memory.rs`](src/app/chat_memory.rs))

Two helpers tie the chat path to the memory writer:

- `build_turn_transcript(app, conv_id, assistant_content)` — formats `USER: …\n\nASSISTANT: …` for the per-turn S3 extractor. Skip conditions match `store_chat_turn`.
- `consolidate_conversation(app, conv_id)` — fires when a conversation closes. Spawns a tokio task that runs `Consolidator::consolidate_run` (per-run triage + decay + cross-scope promotion) using the `WriterHandle`.

Both flow through [`WriterHandle`](../gaviero-core/src/memory/writer.rs) — the panel never touches the SQLite Mutex directly.

---

## 9. Session Persistence

[`app/session.rs`](src/app/session.rs) bridges [`gaviero_core::session_state`](../gaviero-core/src/session_state.rs). `SessionState` carries workspace path, open tabs (`TabState`), current tab, panel modes, terminal visibility, and `StoredConversation` history (each conversation may carry a `ContinuityHandle::CursorThreadId` for native Cursor resume). `ConversationIndex` + `StoredConversation` JSON files live under `.gaviero/state/`. `load_session` on startup, `save_session` on timer and shutdown. The per-folder topology cache lives in `App` (not serialized) — rebuilt asynchronously on workspace open.

---

## 10. Concurrency

Single-threaded UI + async producers. **No `Mutex` in TUI state.** Observer `Arc`s are cloned into core tasks; events flow one-way into the main loop channel. Memory panel destructive operations enqueue `WriterMessage::PanelEdit` and await a 500 ms oneshot ack; until the ack fires the panel renders a pending spinner. Topology builds run on a tokio task and write into the cache through `RwLock::write_owned`; render must tolerate a not-yet-resolved cache entry.

---

## 11. Error Handling

- User-facing failures appear in the status bar or a transient alert.
- Swarm / ACP errors arrive as `Event::SwarmCompleted(failed)` / `Event::AcpTaskCompleted(Err)` and render into the relevant side panel.
- Memory errors surface through `MemoryObserver` events.
- Panic handler in `main.rs` restores the terminal (raw mode off, alternate screen off, cursor visible) before unwinding so a panic does not leave the user terminal in a broken state.

---

## 12. Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering, input.
- `ropey 1.6` — rope buffer.
- `notify 7` — filesystem watch.
- `portable-pty 0.9` + `vt100 0.16` + `tui-term 0.3` — embedded terminal.
- `arboard 3` + `base64` + `png` — clipboard, image paste.
- `unicode-width 0.2` — visual width for wrap layout.
- `streaming-iterator`, `toml`, `tokio-util` — misc.
- [`gaviero-core`](../gaviero-core), [`gaviero-dsl`](../gaviero-dsl).

---

## 13. API Surface

No public library API. Binary entry is [`src/main.rs`](src/main.rs); everything else is crate-private.

---

See [CLAUDE.md](CLAUDE.md) for build, conventions, and rules, and [README.md](README.md) for keybindings, settings, and themes.
