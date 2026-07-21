# gaviero-tui

Interactive terminal editor and workspace for Gaviero. A multi-tab code editor, file tree, git integration, agent chat, swarm dashboard, and embedded terminal in one full-screen TUI.

## Overview

This is the interactive front-end. All execution logic lives in [`gaviero-core`](../gaviero-core/README.md); the TUI handles rendering and input only. It combines:

- **Multi-tab editor** — Ropey rope buffer with syntax highlighting, word wrap, undo/redo, and search.
- **File tree** — navigate and open files from the left panel.
- **Git panel** — stage/unstage, commit, branch management, diff review.
- **Agent chat** — talk to Claude agents with file context, streaming output, and context-pressure indicators.
- **Swarm dashboard** — monitor multi-agent tasks: logs, timing, and cost.
- **Memory panel** — inspect stored memories, query by scope, trigger consolidation, audit retrieval manifests.
- **Search panel** — workspace-wide search with results navigation.
- **Embedded terminal** — a full PTY shell with OSC 133 support.
- **Session restore** — persistent tabs, layout, and conversation history.

## Installation

```bash
cargo build  -p gaviero-tui
cargo run    -p gaviero-tui        # launch editor in the current directory
cargo test   -p gaviero-tui
cargo clippy -p gaviero-tui
```

Binary name: `gaviero`.

## Usage

```bash
gaviero                                        # current directory
gaviero /path/to/repo                          # a specific project
gaviero /path/to/workspace.gaviero-workspace   # a multi-folder workspace
```

On first run you'll be prompted to create a workspace settings file.

## Examples

Open a project, then in the agent chat panel (Alt+A):

```
review src/auth/session.rs for race conditions
```

Switch models and run a workflow mid-session:

```
/model claude:opus
/run workflows/refactor.gaviero "extract the token cache into its own module"
```

Kick off an ad-hoc multi-agent swarm and watch it on the dashboard (Alt+W):

```
/cswarm add end-to-end tests for the billing API
```

## Keybindings

| Context | Keys | Action |
|---|---|---|
| Focus | Alt+1 / Alt+2 / Alt+3 / Alt+4 | Left panel / editor / side panel / terminal |
| Layout | Ctrl+B | Show/hide file tree |
| Layout | Ctrl+P | Show/hide side panel |
| Layout | Ctrl+J / F4 | Toggle bottom terminal panel |
| Layout | F11 | Toggle fullscreen for the current panel |
| Layout | Alt+5 … Alt+0 | Switch layout preset (1–6) |
| Layout | Ctrl+Up / Ctrl+Down | Resize the focused panel (terminal split, chat input) |
| Left panel | Alt+E / Alt+F / Alt+C | Explorer / Find / Changes |
| Side panel | Alt+A / Alt+W / Alt+G / Alt+M | Agent Chat / Swarm / Git / Memory |
| Tabs | Ctrl+T / Ctrl+W | New tab / close tab |
| Tabs | Alt+O / Alt+I | Cycle tabs forward / back |
| Edit | Ctrl+S | Save |
| Edit | Ctrl+Z / Ctrl+Y | Undo / Redo |
| Edit | Ctrl+C / Ctrl+X / Ctrl+V | Copy / Cut / Paste |
| Edit | Ctrl+A | Select all |
| Edit | Ctrl+Left / Ctrl+Right | Word movement |
| Edit | Shift+Arrow / Ctrl+Shift+Arrow | Extend selection / by word |
| Edit | Alt+Up / Alt+Down | Move line up / down |
| Edit | Ctrl+K / Ctrl+D | Delete line / duplicate line |
| Edit | Ctrl+H | Delete word backward |
| Edit | F2 | Rename symbol |
| Edit | F5 / F6 | Format buffer / cycle format level |
| Edit | F7 (also Alt+Z / Alt+Shift+Z) | Toggle word wrap |
| Editor (.md) | Alt+P (also Alt+Shift+P) | Cycle markdown view: source → split → preview |
| Find | Ctrl+F | Find in file |
| Find | F3 | Next in-file match (find bar open) / workspace search (closed) |
| Chat | Alt+Y | Toggle auto-approve for the next tool prompt |
| Merge conflict | F8 / F9 | Next / previous conflict region or file |
| Diff review | `]h` / `[h` | Next / previous hunk |
| Diff review | `a` / `r` | Accept / reject current hunk |
| Diff review | `A` / `R` | Accept / reject all |
| Diff review | `f` / `q` | Finalize (write to disk) / exit review |

F7 is the reliable word-wrap chord on every host; plain Alt+Z is a fallback (NVIDIA's overlay registers it as a global hotkey on some machines, so the terminal never sees it). The status bar shows `[W]` when word wrap is active.

Mouse drag selects text panel-by-panel and copies on release. If a drag instead spans the whole terminal window, your multiplexer is selecting on top of gaviero; under psmux, add `set -g mouse-selection off` to `~/.psmux.conf`.

## Chat Commands

Type these in the agent chat panel:

| Command | Purpose |
|---|---|
| `/model <spec>` | Switch active model (e.g., `claude:sonnet`, `ollama:qwen2.5-coder:7b`) |
| `/run <file.gaviero> [prompt]` | Compile and execute a DSL workflow, with optional runtime prompt |
| `/swarm <task>` | Immediate multi-agent swarm (auto-decomposed) |
| `/cswarm <task>` | Generate a reviewable coordinated plan (`.gaviero` file) |
| `/undo-swarm` | Revert the last swarm result |
| `/remember <text>` | Store a fact in semantic memory (`/remember-here`, `-module`, `-workspace`, `-global` scope it) |
| `/forget <query>` | Soft-delete memories matching the query |
| `/skills [search <q>]` | List loaded skills, or search them semantically |
| `/attach <path>` / `/detach <name\|all>` | Add/remove a file in chat context |
| `/lite` | Send a minimal-context turn (topology kept; outline, memory, impact dropped) |
| `/compact` | Trim conversation history while preserving key context |
| `/clear` | Clear conversation history |

Chat input also supports `$skill` invocation with `$`-prefix autocomplete at ≥2 characters.

## Configuration

The TUI reads workspace settings from this cascade (highest priority first):

1. `.gaviero/settings.json` — project-level settings
2. `.gaviero-workspace` file — multi-folder configuration
3. `~/.config/gaviero/settings.json` — user defaults
4. Built-in defaults

Example `.gaviero/settings.json`:

```json
{
  "editor": { "tabSize": 4, "insertSpaces": true, "wordWrap": false },
  "agent": {
    "model": "claude:sonnet",
    "maxTokens": 16384,
    "ollamaBaseUrl": "http://localhost:11434",
    "availableTools": ["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash"],
    "approvedTools": ["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"],
    "permissions": {
      "bash": {
        "denylist": ["terraform destroy", "npm publish", "git push --force"],
        "allowlist": ["cargo check", "cargo test", "git status", "rg "]
      }
    },
    "coordinator": { "model": "claude:opus" }
  },
  "memory": { "namespace": "my-project" }
}
```

Language-specific overrides use bracket syntax: `"[rust]": { "editor.tabSize": 4 }`.

## API / Architecture

The TUI implements three observer traits from `gaviero-core` — `WriteGateObserver`, `AcpObserver`, `SwarmObserver`. Each impl holds an event-channel sender, so core callbacks become `Event` variants handled on the main loop.

**Event-loop golden rule:** `draw → recv event → handle → repeat`. Render is pure; mutation happens only in `handle`. No background task mutates the `App` struct directly — all state changes flow through a single `mpsc` event channel, and the TUI holds no `Mutex`.

Module layout:

- `app.rs` + `app/` — `App` struct, layout, focus, event dispatch, observer wiring, slash-command dispatch, context-pressure render.
- `event.rs` — event variants from crossterm / notify / tick / observer callbacks; Windows paste-burst coalescer.
- `keymap.rs` — keybindings (Ctrl = editor, Alt = workspace layering).
- `platform.rs` — platform-specific terminal workarounds (VT mouse passthrough, bracketed-paste gating, AltGr detection).
- `editor/` — Ropey buffer, viewport/gutter, tree-sitter highlight, markdown rendering, diff overlay, word-wrap layout.
- `panels/` — `file_tree`, `agent_chat`, `swarm_dashboard`, `git_panel`, `terminal`, `search`, `memory_panel`, `status_bar`.
- `widgets/` — tabs, scrollbar, scroll state, text input.
- `theme.rs` — One Dark palette + timing constants.

## Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering, input
- `ropey 1.6` — rope buffer; `notify 7` — filesystem watcher
- `portable-pty 0.9` + `vt100 0.16` — embedded terminal
- `arboard 3` + `png` — clipboard and image paste
- `gaviero-core`, `gaviero-dsl` — runtime + DSL compilation

## See Also

- [CLAUDE.md](CLAUDE.md) — event loop, panel patterns, observer bridge conventions
- [Root README](../../README.md) — full feature overview and settings reference
- [`crates/gaviero-core/README.md`](../gaviero-core/README.md) — the runtime the TUI drives

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
