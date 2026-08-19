# gaviero-tui

## Overview

Interactive terminal editor and workspace for Gaviero. A multi-tab code editor, file tree, git integration, agent chat, swarm dashboard, and embedded terminal in one full-screen TUI. All execution logic lives in [`gaviero-core`](../gaviero-core/README.md); this crate handles rendering and input only.

## Installation

```bash
cargo build  -p gaviero-tui
cargo run    -p gaviero-tui        # launch in current directory
cargo test   -p gaviero-tui
cargo clippy -p gaviero-tui
```

Binary name: `gaviero`.

## Usage

```bash
gaviero                                        # current directory
gaviero /path/to/repo                          # single-folder workspace
gaviero /path/to/workspace.gaviero-workspace   # multi-folder workspace
```

On first run you may be prompted to create a workspace settings file.

**Panel focus:** Alt+1/2/3/4 (left/editor/side/terminal). **Side panels:** Alt+A (chat), Alt+W (swarm), Alt+G (git), Alt+M (memory). **Left panel:** Alt+E (explorer), Alt+F (find), Alt+C (changes).

## Examples

**Review a file** (agent chat, Alt+A):

```
review src/auth/session.rs for race conditions
```

**Switch model and run a workflow:**

```
/model claude:opus
/run workflows/refactor.gaviero "extract the token cache into its own module"
```

**Ad-hoc multi-agent swarm** (watch on swarm dashboard, Alt+W):

```
/cswarm add end-to-end tests for the billing API
```

**Write Gate review** — when an agent proposes changes, a diff overlay opens:

| Key | Action |
|---|---|
| `]h` / `[h` | Next / previous hunk |
| `a` / `r` | Accept / reject current hunk |
| `A` / `R` | Accept / reject all |
| `f` / `q` | Finalize (write to disk) / exit |

### Chat commands

| Command | Purpose |
|---|---|
| `/model <spec>` | Switch model (`claude:sonnet`, `deepseek:deepseek-v4-pro`, …) |
| `/run <file.gaviero> [prompt]` | Compile and execute a DSL workflow |
| `/swarm <task>` | Immediate multi-agent swarm |
| `/cswarm <task>` | Generate a reviewable coordinated plan |
| `/undo-swarm` | Revert the last swarm result |
| `/remember <text>` | Store a fact (`-here`, `-module`, `-workspace`, `-global` scope it) |
| `/forget <query>` | Soft-delete matching memories |
| `/skills [search <q>]` | List or search loaded skills |
| `/attach <path>` / `/detach` | Add/remove file context |
| `/lite` | Minimal-context turn (topology only) |
| `/compact` / `/clear` | Trim or clear conversation history |

Chat input supports `$skill` invocation with `$`-prefix autocomplete. Full slash inventory: [CLAUDE.md](CLAUDE.md).

### Keybindings (selected)

| Keys | Action |
|---|---|
| Ctrl+B / Ctrl+P / Ctrl+J | Toggle file tree / side panel / terminal |
| Ctrl+S / Ctrl+Z / Ctrl+Y | Save / undo / redo |
| Ctrl+F / F3 | Find in file / workspace search |
| F5 / F7 | Format buffer / toggle word wrap |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Alt+Up/Down (Ctrl+Up/Down fallback) | Resize terminal split / chat input height |
| Ctrl+Alt+Left/Right | Resize explorer / editor / side panel widths |

F7 is the reliable word-wrap chord on every host. See [Root README](../../README.md) for the full editor keybinding tables.

## Configuration

Settings cascade (highest priority first):

1. `.gaviero/settings.json`
2. `.gaviero-workspace` file
3. `~/.config/gaviero/settings.json`
4. Built-in defaults

```json
{
  "editor": { "tabSize": 4, "insertSpaces": true, "wordWrap": false },
  "agent": {
    "model": "claude:sonnet",
    "maxTokens": 16384,
    "ollamaBaseUrl": "http://localhost:11434",
    "coordinator": { "model": "claude:opus" }
  },
  "memory": { "namespace": "my-project" },
  "skills": { "extraRoots": ["~/.claude/skills", "~/.codex/skills"] }
}
```

Language-specific overrides: `"[rust]": { "editor.tabSize": 4 }`.

`skills.extraRoots` is an optional array of extra skill directories (default `[]`, `~`/`~/` expanded). Typical values are `"~/.claude/skills"` and `"~/.codex/skills"`. A configured path that does not exist or is unreadable is shown as a chat system message. Unqualified `$name` still prefers a workspace/repo/global skill over a foreign one; collisions complete as `$source/name`.

### Agent notifications

Two chat milestones announce themselves, each with its own `enabled` / `sound` / `desktop` / `statusBar` switches (all default `true`):

| Setting group | Fires when |
|---|---|
| `notifications.agentFinished.*` | The turn ended — an answer (or an error) is on screen |
| `notifications.agentWaiting.*` | The turn is blocked on you — a tool permission prompt or an `AskUserQuestion` |

The two use different sounds, glyphs, and banner colours (`✓` green vs `?` amber) so they are distinguishable without looking.

Focus handling differs per channel, by design:

- **Sound ignores focus** — it fires even when gaviero is backgrounded or minimized, which is the case the alert exists for.
- **Desktop toasts fire only while the terminal is unfocused** — with gaviero on screen the status-bar banner already carries the news. (No-op on Windows: see `spawn_desktop_notification`.)
- **The status-bar banner and fullscreen toast** are drawn while you are looking, for 8s.

`notifications.sound.style` picks how the sound is produced:

| Value | Behaviour |
|---|---|
| `"auto"` (default) | Win32 system sound on Windows, terminal BEL elsewhere |
| `"bell"` | Terminal BEL (`\x07`) only |
| `"system"` | Platform system sound, falling back to BEL where none exists |
| `"both"` | BEL *and* the system sound |

Windows defaults to the system sound because BEL has to survive ConPTY plus any multiplexer, and Windows Terminal's `bellStyle` can silence it outright. Set `"bell"` if you route notifications through your terminal, or `"both"` if either channel might be dropped.

```json
{
  "notifications": {
    "agentFinished": { "sound": true, "desktop": false },
    "agentWaiting": { "sound": true },
    "sound": { "style": "both" }
  }
}
```

## API

The TUI is a binary, not a library. Internally it implements three observer traits from `gaviero-core` — `WriteGateObserver`, `AcpObserver`, `SwarmObserver` — bridging core callbacks into an `mpsc` event channel.

**Event-loop rule:** `draw → recv event → handle → repeat`. No background task mutates `App` directly.

| Module | Role |
|---|---|
| `app.rs` + `app/` | `App` struct, layout, focus, slash commands |
| `editor/` | Ropey buffer, tree-sitter highlight, diff overlay, word wrap |
| `panels/` | `agent_chat`, `swarm_dashboard`, `git_panel`, `memory_panel`, `terminal`, … |
| `keymap.rs` / `event.rs` | Keybindings and event variants |
| `platform.rs` | Windows/ConPTY, AltGr, paste coalescing |

## See Also

- [CLAUDE.md](CLAUDE.md) — event loop, panel patterns, full slash list
- [Root README](../../README.md) — feature overview
- [gaviero-core](../gaviero-core/README.md) — runtime the TUI drives

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
