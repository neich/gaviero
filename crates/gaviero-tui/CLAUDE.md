# gaviero-tui

Full-screen TUI editor binary. Rendering and input handling only — all logic delegates to `gaviero-core`.

## Build & Test

```bash
cargo test -p gaviero-tui
cargo clippy -p gaviero-tui
cargo run -p gaviero-tui       # launches the editor
```

Binary name: `gaviero`

## Module Overview

| Module | Purpose |
|---|---|
| `app.rs` | Main `App` struct, layout, focus management, event dispatch (~5000 lines) |
| `event.rs` | `Event` enum (43+ variants), `EventLoop` (crossterm/watcher/tick/terminal) |
| `keymap.rs` | `Action` enum, keybinding definitions. Ctrl=editor, Alt=workspace layering |
| `theme.rs` | One Dark color constants, timing constants |
| `editor/` | `buffer` (Ropey), `view`, `diff_overlay`, `highlight`, `markdown` |
| `panels/` | `file_tree`, `agent_chat`, `swarm_dashboard`, `git_panel`, `terminal`, `status_bar`, `search` |
| `widgets/` | `tabs`, `scrollbar`, `scroll_state`, `text_input`, `render_utils` |

## Observer Bridge

TUI implements `WriteGateObserver`, `AcpObserver`, `SwarmObserver` from `gaviero-core::observer`. Each holds a clone of the event channel sender — core callbacks become `Event` variants processed in the main loop.

## Key Dependencies

- `ratatui 0.30` + `crossterm 0.29` — terminal rendering
- `ropey` — rope-based text buffer
- `notify` — filesystem watching
- `portable-pty` + `vt100` + `tui-term` — embedded terminal
- `gaviero-dsl` — DSL compilation from editor

## Conventions

- Single event channel: all external events flow through one `mpsc::unbounded_channel<Event>`. No background task mutates `App` directly.
- Main loop: `draw → recv → handle → repeat`.
- Diff overlay keybinds: `]h`/`[h` navigate hunks, `a`/`r` accept/reject, `A`/`R` all, `f` finalize, `q` exit.
