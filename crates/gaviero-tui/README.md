# gaviero-tui

Interactive terminal editor and workspace for Gaviero. Multi-tab code editor, file tree, git integration, agent chat, swarm dashboard, and embedded terminal all in one full-screen TUI.

This is the interactive front-end. All execution logic lives in `gaviero-core`; the TUI handles rendering and input only.

## Installation & Build

```bash
cargo build -p gaviero-tui
cargo run -p gaviero-tui              # launch editor
cargo test -p gaviero-tui
cargo clippy -p gaviero-tui
```

Binary name: `gaviero`

## Overview

The TUI combines multiple editing and collaboration features:

- **Multi-tab editor** — Ropey-based rope buffer with syntax highlighting, undo/redo, search
- **File tree** — Navigate and open files from a left panel
- **Git panel** — Stage/unstage, commit, branch management, diff review
- **Agent chat** — Talk to Claude agents with file context and streaming output
- **Swarm dashboard** — Monitor multi-agent tasks, view logs, check timing and cost
- **Search panel** — Workspace-wide search with results navigation
- **Embedded terminal** — Full PTY shell with OSC 133 support
- **Session restore** — Persistent tabs, layout, and conversation history

## Running the Editor

```bash
gaviero                    # current directory
gaviero /path/to/repo      # specific project
gaviero /path/to/workspace.gaviero-workspace  # multi-folder workspace
```

On first run, you'll be prompted to create a workspace settings file.

## Chat Commands

Type these in the agent chat panel to control execution:

| Command | Purpose |
|---|---|
| `/model <spec>` | Switch active model (e.g., `sonnet`, `ollama:qwen2.5-coder:7b`) |
| `/run <file.gaviero>` | Compile and execute a DSL workflow |
| `/run <file> <prompt>` | Execute with runtime prompt substitution |
| `/swarm <task>` | Immediate multi-agent swarm (auto-decomposed) |
| `/cswarm <task>` | Generate a reviewable coordinated plan (.gaviero file) |
| `/undo-swarm` | Revert the last swarm result |
| `/remember <text>` | Store a fact in semantic memory |
| `/attach <path>` | Include a file in chat context |
| `/detach <name\|all>` | Remove attachments |

## Configuration

The TUI reads workspace settings from this cascade:

1. `.gaviero/settings.json` — project-level settings
2. `.gaviero-workspace` file — multi-folder configuration
3. `~/.config/gaviero/settings.json` — user defaults
4. Built-in defaults

Example `.gaviero/settings.json`:

```json
{
  "editor": {
    "tabSize": 4,
    "insertSpaces": true
  },
  "agent": {
    "model": "sonnet",
    "maxTokens": 16384,
    "ollamaBaseUrl": "http://localhost:11434",
    "coordinator": {
      "model": "opus"
    }
  },
  "memory": {
    "namespace": "my-project"
  }
}
```

## API / Architecture

The TUI implements three observer interfaces from `gaviero-core`:

- `WriteGateObserver` — receives proposal accept/reject events
- `AcpObserver` — receives agent chat progress events
- `SwarmObserver` — receives multi-agent coordination events

Each observer sends events to the main event loop as `Event` variants, which are processed synchronously.

**Event loop** — single-threaded: `draw → recv event → handle → repeat`

No background tasks mutate the `App` struct directly. All state changes flow through the event channel.

### Key keybindings

| Context | Keys | Action |
|---|---|---|
| Editor | Ctrl+S | Save |
| Editor | Ctrl+Z / Ctrl+Y | Undo / Redo |
| Editor | Ctrl+F | Find in file |
| Workspace | Alt+1/2/3/4 | Focus left / editor / right / terminal |
| Workspace | Alt+a / Alt+w / Alt+g | Chat / Swarm / Git panel |
| Diff review | `]h` / `[h` | Next / previous hunk |
| Diff review | `a` / `r` | Accept / reject current hunk |
| Diff review | `A` / `R` | Accept / reject all |
| Diff review | `f` | Finalize (write to disk) |

See the root [README.md](../../README.md) for a complete keybinding reference.
