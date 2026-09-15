# gaviero-tui — Architecture

Full-screen terminal editor. Rendering + event routing only — runtime logic lives in [`gaviero-core`](../gaviero-core). Binary: `gaviero`.

Conventions and slash inventory: [CLAUDE.md](CLAUDE.md). Keybindings / settings: [README.md](README.md).

---

## Topology

```
gaviero-core ◄── observer traits + WriterHandle
gaviero-dsl  ◄── compile_file (/run)
        ▲
┌───────┴────────────────────────┐
│          gaviero-tui           │
│  App ◄── Event loop (single    │
│   │      mpsc::unbounded)      │
│   └─ render / handle_event     │
│  sources: crossterm | notify | │
│           tick | PTY | observers│
└────────────────────────────────┘
```

Workspace dispatch in [`main.rs`](src/main.rs): directory → [`Workspace::single_folder`](../gaviero-core/src/workspace.rs); `*.gaviero-workspace` → [`Workspace::load`](../gaviero-core/src/workspace.rs); `--workspace <dir>` reuses the workspace file inside `<dir>` or creates one.

Before that dispatch, [`setup.rs`](src/setup.rs) runs the first-run wizard when the target is unconfigured and stdin is a TTY. It enters and leaves its own raw-mode/alternate-screen session, writes `.gaviero/settings.json` (+ the `.gaviero-workspace` file in `--workspace` mode), and returns the launch target `main` then loads. Opt-in provider configs are synthesized afterwards, once the `Workspace` exists.

---

## Modules

```
gaviero-tui/src/
├─ main.rs              Terminal setup, panic restore, workspace dispatch
├─ app.rs               App (state + layout + focus)
├─ event.rs             Event enum + source plumbing (+ Windows paste coalescer)
├─ keymap.rs            Action enum, chords (F7 word-wrap primary)
├─ platform.rs          ConPTY / AltGr / Ctrl+C forwarder — all OS quirks
├─ theme.rs             Palette + timing
├─ editor/              buffer, view, wrap, highlight, markdown, diff, diff_overlay
├─ panels/              file_tree, agent_chat, history_panel, git, terminal,
│                       search, memory_panel, status_bar, chat_markdown
├─ widgets/             tabs, scrollbar, scroll_state, text_input, render_utils
└─ app/
   ├─ controller.rs     Top-level event → action dispatch
   ├─ layout.rs / render.rs / left_panel.rs / side_panel.rs / review.rs
   ├─ commands.rs       Slash commands (authoritative list — see CLAUDE.md)
   ├─ editing.rs        Editor + find-bar (viewport width for wrap)
   ├─ session.rs        session_state bridge + per-folder topology cache
   ├─ chat_memory.rs    Turn transcript + consolidator → WriterHandle
   ├─ remote.rs         Remote command reducers (every tab by conv_id)
   ├─ remote_setup.rs   Sidecar bootstrap, `/remote`, machine token
   ├─ state.rs          Shared enums
   └─ observers.rs      WriteGate / Acp / Memory / Manifest / Mcp → Event (+ history capture)
```

---

## Abstractions

### `App` ([`app.rs`](src/app.rs))

Owns tabs, panels, focus, theme, workspace, optional `MemoryStores` + `WriterHandle`, topology cache, event sender. Sole mutator via `handle_event`.

### Focus / panels

```rust
enum Focus { Editor, FileTree, SidePanel, Terminal }
enum LeftPanelMode  { FileTree, Search, Changes, Review }
enum SidePanelMode  { AgentChat, HistoryPanel, GitPanel, MemoryPanel }
```

### Observer bridge ([`app/observers.rs`](src/app/observers.rs))

Implements core observer traits; each holds `mpsc::UnboundedSender<Event>`. Includes `CursorSessionStarted` → `SessionLedger` continuity. `TuiAcpObserver` (built per turn, carrying its `turn_id`) and `TuiMcpObserver` also write to the app's single `gaviero_core::history::HistoryRecorder` — see History panel below. The TUI no longer implements `SwarmObserver`; swarms run from `gaviero-cli --script`.

### History panel ([`panels/history_panel.rs`](src/panels/history_panel.rs))

`Alt+h`. Per turn: full prompt, tool calls (args + result, or summary-only), MCP calls (request + response), the memory call (manifest + injected block), and token totals. Capture: [`dispatch_prompt_core`](src/app/side_panel.rs) opens the turn with the verbatim prompt; the turn's task records the memory injection, bootstrap size, and turn end **by `turn_id`** (a conversation can start its next turn before the previous task reports finished); `TuiAcpObserver` records tools / assistant output / provider usage; `TuiMcpObserver` records MCP calls (attributed only when one conversation is streaming). The panel reads `<root>/.gaviero/history/turns.ndjson` off the loop (`Event::HistoryLoaded`) and never writes. Per-item token numbers are `~` estimates with the estimator named in the footer; exact numbers are provider-reported.

### Topology cache ([`app/session.rs`](src/app/session.rs))

`get_or_build_topology_cached` runs [`build_folder_topology`](../gaviero-core/src/repo_map/topology.rs) on a background task; UI must render before the cache resolves.

---

## Data Flow

### Event loop

```
crossterm | notify | tick | PTY | observers
              │
              ▼
   mpsc::unbounded_channel<Event>
              │
              ▼
   loop { draw → recv → handle_event → drain up to 64 → quit? }
```

**Golden rule:** no background task mutates `App`. Render is pure.

### Layout (5 areas)

```
Tab bar
Left panel │ Editor (ropey + wrap + highlight) │ Side panel
Embedded PTY terminal
Status bar (mode | file | branch | Wrap | agent)
```

### Slash commands

Authoritative inventory: [`app/commands.rs`](src/app/commands.rs) + chat helpers in [`panels/agent_chat.rs`](src/panels/agent_chat.rs). Groups: session (`/model`, `/effort`, `/autoapprove`/`/yolo`, …), context (`/lite`, `/inject`, `/no-inject`, `/context mode …`, `/namespace`), memory (`/remember*`, `/forget*`, `/restore`, `/reembed`, `/sleep`, `/consolidate-session`), MCP (`/mcp`, `/mcp probe`), skills (`/skills`, `$skill`), attachments (`/attach`, `/detach`). Do not maintain a second full table here — see [CLAUDE.md](CLAUDE.md).

### Chat ↔ memory ([`app/chat_memory.rs`](src/app/chat_memory.rs))

`build_turn_transcript` + `consolidate_conversation` enqueue through [`WriterHandle`](../gaviero-core/src/memory/writer.rs) only.

---

## Concurrency

Single-threaded UI + async producers. **No `Mutex` in TUI state.** Observers clone `Arc` senders into core tasks. Memory panel destructive ops enqueue `WriterMessage::PanelEdit` with `ack: None`. Topology cache uses `RwLock` write from a tokio task.

---

## Error Handling

- User-facing failures → status bar / transient alert.
- ACP errors → `AcpTaskCompleted` / `AgentTurnFinished` → side panel.
- Memory errors → `MemoryObserver` events.
- Panic handler in [`main.rs`](src/main.rs) restores terminal (raw mode off, alt screen off, cursor on) before unwind.
- Merge conflicts: F8/F9 navigate regions via [`git_conflict`](../gaviero-core/src/git_conflict.rs).

---

## API

No public library API. Entry: [`src/main.rs`](src/main.rs). Dependencies: ratatui, crossterm, ropey, notify, portable-pty, vt100, unicode-width, arboard, windows-sys (Windows), gaviero-core, gaviero-dsl — see [CLAUDE.md](CLAUDE.md).
