# gaviero-tui

Full-screen terminal editor. Rendering + input only — all logic delegates to `gaviero-core`.

Binary: `gaviero` ([src/main.rs](src/main.rs)).

## Build & Test

```bash
cargo test -p gaviero-tui
cargo clippy -p gaviero-tui
cargo run -p gaviero-tui        # launch editor in current dir
cargo run -p gaviero-tui -- path/to/repo
cargo run -p gaviero-tui -- name.gaviero-workspace
```

## Architecture

- [`app.rs`](src/app.rs) + [`app/`](src/app) — `App` struct, layout, focus, event dispatch, observer wiring, chat-memory bridge ([`app/chat_memory.rs`](src/app/chat_memory.rs)), per-folder topology cache built asynchronously ([`app/session.rs`](src/app/session.rs)), slash-command dispatch ([`app/commands.rs`](src/app/commands.rs)), context-pressure + bootstrap-tokens render ([`app/render.rs`](src/app/render.rs)).
- [`event.rs`](src/event.rs) — event variants from crossterm / notify / tick / core observer callbacks.
- [`keymap.rs`](src/keymap.rs) — keybindings: Ctrl = editor, Alt = workspace layering. `Alt+Z` toggles word wrap.
- [`editor/`](src/editor) — Ropey buffer ([`buffer.rs`](src/editor/buffer.rs)), viewport + gutter ([`view.rs`](src/editor/view.rs)), tree-sitter highlight ([`highlight.rs`](src/editor/highlight.rs)), markdown rendering ([`markdown.rs`](src/editor/markdown.rs)), diff-overlay state ([`diff_overlay.rs`](src/editor/diff_overlay.rs)), LCS line diff for diff-view buffers ([`diff.rs`](src/editor/diff.rs)), visual-line layout for word wrap ([`wrap.rs`](src/editor/wrap.rs)).
- [`panels/`](src/panels) — `file_tree`, `agent_chat` (slash commands + context-pressure + bootstrap-tokens indicators), `swarm_dashboard`, `git_panel`, `terminal`, `search`, `memory_panel`, `status_bar` (mode / file / branch / agent / word-wrap indicator), `chat_markdown`.
- [`widgets/`](src/widgets) — tabs, scrollbar, scroll state, text input, render utils.
- [`theme.rs`](src/theme.rs) — One Dark palette + timing constants.

## Observer Bridge

The TUI implements `WriteGateObserver`, `AcpObserver`, `SwarmObserver` from [`gaviero_core::observer`](../gaviero-core/src/observer.rs). Each impl holds an event-channel sender — core callbacks become `Event` variants processed on the main loop. **No background task mutates `App` directly.**

## Slash Commands

Dispatched in [`app/commands.rs`](src/app/commands.rs) and [`panels/agent_chat.rs`](src/panels/agent_chat.rs). The active set:

`/swarm`, `/cswarm`, `/undo-swarm`, `/run`, `/model` (set runtime model), `/compact` (compact chat context), `/clear` (alias `/reset`), `/lite` (alias `/minimal` — minimal-context turn: keeps `<repo_topology>`, drops `<repo_outline>` + memory + impact), `/remember`, `/remember-here`, `/remember-module`, `/remember-workspace`, `/remember-global`, `/forget`, `/forget-scope`, `/forget-type`, `/forget-source`, `/forget-history`, `/restore`, `/attach`, `/detach`, `/help`.

## Conventions

- **Single event channel.** All external sources (crossterm input, notify watchers, ticks, observer callbacks) funnel into one `mpsc::unbounded_channel<Event>`.
- **Event-loop golden rule.** `draw → recv → handle → repeat`. Render is pure; mutation only happens in `handle`.
- **No `Mutex` in the TUI.** State changes go through the event loop.
- Diff overlay keys: `]h` / `[h` navigate; `a` / `r` accept/reject; `A` / `R` all; `f` finalize; `q` exit.
- Merge conflicts: F8 / F9 next/previous region (editor) or conflict file (Changes panel); save stages when markers are gone.
- Editing actions that depend on wrapped layout receive viewport width via [`app/editing.rs`](src/app/editing.rs); never compute visual position outside the editor module.

## Rules

- **Never call core APIs from a panel render path.** Panels read from `App` state; all mutation goes through the event loop.
- **Never hold a lock across `.await` on the UI side.** The TUI is single-task; if locking is tempting, use a channel instead.
- **Topology prefetch is best-effort.** [`app/session.rs`](src/app/session.rs) and [`app/side_panel.rs`](src/app/side_panel.rs) build the per-folder topology asynchronously — UI must render correctly even before the cache resolves.
- **Slash-command parsing is line-prefix only.** Use `strip_prefix("/cmd")` patterns ([`app/commands.rs`](src/app/commands.rs)); do not parse with regexes.

## Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering, input.
- `ropey 1.6` — rope buffer.
- `notify 7` — filesystem watcher.
- `portable-pty 0.9` + `vt100 0.16` + `tui-term 0.3` — embedded terminal.
- `arboard 3` + `base64` + `png` — clipboard, image paste.
- `unicode-width 0.2` — visual width for wrap.
- `streaming-iterator`, `toml`, `tokio-util` — misc.
- `gaviero-core`, `gaviero-dsl` — runtime + DSL compilation.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — event loop, layout, panel patterns, observer bridge.
- [README.md](README.md) — keybindings, settings cascade, themes.
