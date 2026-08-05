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
cargo run -p gaviero-tui -- --workspace path/to/dir   # multi-folder from a directory
```

Workspace dispatch: directory → `Workspace::single_folder`; `*.gaviero-workspace` → `Workspace::load`; `--workspace <dir>` → the `*.gaviero-workspace` already in `<dir>`, else one the setup wizard writes ([src/main.rs](src/main.rs)).

**First-run setup** ([src/setup.rs](src/setup.rs)) runs *before* the editor, owning its own raw-mode session so the C1 consent prompt after it still reads plain stdin. Triggers when the target has no `.gaviero/settings.json` (folder mode) or no `*.gaviero-workspace` (`--workspace`), and never without a TTY. Steps: agent profile (full / restricted — restricted drops `Bash` from `agent.availableTools`), workspace members (`--workspace` only), provider configs (yes grants `mcp.gavieroServer.codexTrust` so `.codex/config.toml` is written), confirm. Existing files are never overwritten.

## Architecture

- [`app.rs`](src/app.rs) + [`app/`](src/app) — `App`, layout, focus, observers, chat-memory ([`app/chat_memory.rs`](src/app/chat_memory.rs)), topology cache ([`app/session.rs`](src/app/session.rs)), slash commands ([`app/commands.rs`](src/app/commands.rs)), render ([`app/render.rs`](src/app/render.rs)).
- [`event.rs`](src/event.rs) — crossterm / notify / tick / observer events; Windows paste-burst coalescer.
- [`keymap.rs`](src/keymap.rs) — Ctrl = editor, Alt = workspace. **F7** = word-wrap primary (`Alt+Shift+Z` fallback — NVIDIA steals plain `Alt+Z`). `Ctrl+Up/Down` when hosts steal `Alt+arrows`. `Ctrl+Alt+Left/Right` resize explorer/editor/side widths (`Alt+Left/Right` reserved for tmux/psmux).
- [`platform.rs`](src/platform.rs) — all platform quirks (ConPTY mouse, AltGr, Ctrl+C forwarder). New quirks go here, not inline.
- [`setup.rs`](src/setup.rs) — pre-TUI first-run wizard; writes `.gaviero/settings.json`, the `.gaviero-workspace` file, and (opt-in) the Claude/Codex/Cursor MCP configs. Runs before `App` exists, so it holds no `App` state.
- [`editor/`](src/editor) — buffer, view, highlight, markdown, diff overlay, LCS diff, wrap.
- [`panels/`](src/panels) — file tree, agent chat, swarm dashboard, git, terminal, search, memory, status bar.
- [`widgets/`](src/widgets), [`theme.rs`](src/theme.rs).

**Observer bridge:** implements `WriteGateObserver`, `AcpObserver`, `SwarmObserver` from [`gaviero_core::observer`](../gaviero-core/src/observer.rs). Each holds an event-channel sender. **No background task mutates `App` directly.**

**Authoritative slash list:** [`app/commands.rs`](src/app/commands.rs) (and chat helpers in [`panels/agent_chat.rs`](src/panels/agent_chat.rs)). Groups: session (`/model`, `/effort`, `/autoapprove`/`/yolo`, …), context (`/lite`, `/inject`, `/context mode …`), swarm, memory, skills (`/skills`, `$skill`). Do not maintain a second inventory in ARCHITECTURE.md — point here.

## Conventions

- **Single event channel.** All external sources funnel into one `mpsc::unbounded_channel<Event>`.
- **Event-loop golden rule.** `draw → recv → handle → repeat`. Render is pure; mutation only in `handle`.
- **No `Mutex` in the TUI.** State changes go through the event loop.
- Diff overlay: `]h`/`[h` navigate; `a`/`r` accept/reject; `A`/`R` all; `f` finalize; `q` exit.
- Merge conflicts: F8/F9 next/previous region; save stages when markers are gone ([`gaviero_core::git_conflict`](../gaviero-core/src/git_conflict.rs)).
- Wrapped-layout editing receives viewport width via [`app/editing.rs`](src/app/editing.rs); never compute visual position outside the editor module.

## Rules

- **Never call core APIs from a panel render path.** Panels read `App` state; mutation goes through the event loop.
- **Never hold a lock across `.await` on the UI side.** Prefer a channel.
- **Topology prefetch is best-effort.** UI must render before the cache resolves ([`app/session.rs`](src/app/session.rs)).
- **Slash-command parsing is line-prefix only.** Use `strip_prefix("/cmd")` ([`app/commands.rs`](src/app/commands.rs)); no regex.

## Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering, input.
- `ropey 1.6` — rope buffer.
- `notify 7` — filesystem watcher.
- `portable-pty 0.9` + `vt100 0.16` — embedded terminal.
- `arboard 3` + `base64` + `png` — clipboard / image paste.
- `unicode-width 0.2` — visual width for wrap.
- `windows-sys 0.59` (Windows only) — Ctrl+C forwarder ([`platform.rs`](src/platform.rs)).
- `embed-manifest 1.4` (build) — Windows application manifest.
- `gaviero-core`, `gaviero-dsl` — runtime + DSL compilation.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — event loop, layout, panel patterns, observer bridge.
- [README.md](README.md) — keybindings, settings cascade, themes.
