# gaviero-tui

Full-screen TUI editor. Rendering + input only — all logic delegates to `gaviero-core`.

Binary: `gaviero`

## Build & Test

```bash
cargo test -p gaviero-tui
cargo clippy -p gaviero-tui
cargo run -p gaviero-tui       # launch editor
```

## Module Structure

- `app.rs`, `app/` — `App` struct, layout, focus, event dispatch, observer wiring, chat-memory bridge (`app/chat_memory.rs`)
- `event.rs` — event variants from crossterm / notify / tick / core observer callbacks
- `keymap.rs` — keybindings: Ctrl = editor, Alt = workspace layering
- `editor/` — Ropey buffer, view, diff overlay, syntax highlight
- `panels/` — `file_tree`, `agent_chat`, `swarm_dashboard`, `git_panel`, `terminal`, `search`, `memory_panel` (memory inspection / management), `status_bar`, `chat_markdown`
- `widgets/` — tabs, scrollbar, text input, render utils
- `theme.rs` — One Dark colors, timing constants

## Observer Bridge

TUI implements `WriteGateObserver`, `AcpObserver`, `SwarmObserver` from `gaviero-core`. Each holds an event channel sender — core callbacks become `Event` variants processed on the main loop. No background task mutates `App` directly.

## Conventions

- Single event channel: all external sources → one `mpsc::unbounded_channel<Event>`.
- Main loop: `draw → recv → handle → repeat`.
- Diff overlay keys: `]h`/`[h` navigate, `a`/`r` accept/reject, `A`/`R` all, `f` finalize, `q` exit.

## Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering
- `ropey` — rope buffer
- `notify` — filesystem watch
- `portable-pty` + `vt100` + `tui-term` — embedded terminal
- `gaviero-core`, `gaviero-dsl` — runtime + DSL compilation

## See Also

[ARCHITECTURE.md](ARCHITECTURE.md) — event loop, layout, panel patterns.
