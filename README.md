# Gaviero

A terminal code editor built for working with AI coding agents. Gaviero gives you a full editing environment — file tree, syntax highlighting, git integration, embedded terminal — alongside a conversation panel where you chat with Claude agents that can read and modify your code. Every change an agent proposes passes through an interactive review before it touches disk.

There is also a headless CLI for running agent tasks from scripts or CI.

## Quick start

```
cargo build --release
./target/release/gaviero ~/my-project
```

The editor opens with the file tree on the left, your code in the center, and an agent chat panel on the right.

## The editor

Gaviero is a self-contained terminal editor. You can use it for everyday editing without ever touching the AI features.

### Navigation

| Key | What it does |
|---|---|
| Alt+1 / Alt+2 / Alt+3 / Alt+4 | Focus left panel / editor / side panel / terminal |
| Ctrl+b | Show/hide file tree |
| Ctrl+p | Show/hide side panel |
| Ctrl+j / F4 | Show/hide terminal |
| Ctrl+t / Ctrl+w | New tab / close tab |
| Alt+[ / Alt+] | Cycle tabs |
| F11 | Toggle fullscreen for current panel |
| Alt+Shift+1..6 | Switch layout preset |

### Editing

| Key | What it does |
|---|---|
| Ctrl+s | Save |
| Ctrl+z / Ctrl+y | Undo / Redo |
| Ctrl+c / Ctrl+x / Ctrl+v | Copy / Cut / Paste |
| Ctrl+a | Select all |
| Ctrl+Left / Ctrl+Right | Word movement |
| Shift+Arrow | Extend selection |
| Ctrl+Shift+Left / Ctrl+Shift+Right | Extend selection by word |
| Ctrl+k | Delete line |
| Ctrl+d | Duplicate line |
| Alt+Up / Alt+Down | Move line up/down |
| Ctrl+h or Ctrl+Backspace | Delete word backward |
| Ctrl+Delete | Delete to end of line |
| F5 | Format buffer |

### Finding text

**In the current file** — press **Ctrl+F**. A find bar appears at the top of the editor. Type your query and the editor highlights all matches and jumps to the first one. Press **Enter** or **F3** to cycle through matches, **Up** to go backward. Press **Esc** to close.

**Across the workspace** — press **F3** (without the find bar open) to search the word under the cursor across all project files. Or switch to the **Search** panel (Shift+Right from the file tree, or F7) and type directly into the search input. Results update as you type. Press Enter on a result to open the file at that line.

### Side panels

Switch between side panels with Alt+letter:

| Key | Panel |
|---|---|
| Alt+a | Agent Chat |
| Alt+w | Swarm Dashboard |
| Alt+g | Git |

### Left panel modes

Switch between left panel views with Alt+letter:

| Key | Mode |
|---|---|
| Alt+e | Explorer (file tree) |
| Alt+f | Find (workspace search) |
| Alt+c | Changes (git diff list) |

Each shortcut shows the left panel if hidden, switches to the requested mode, and focuses it.

### Git panel

The git panel (Ctrl+3) provides staging, committing, and branch management without leaving the editor:

- **s** / **u** — stage / unstage the selected file
- **c** — commit with the message in the input field
- **a** — amend the last commit
- **b** — open branch picker with filtering
- **d** — discard changes
- Enter on a file shows its diff in the editor

### Embedded terminal

Ctrl+J (or F4) opens a terminal panel at the bottom. The terminal is a full PTY — run builds, tests, git commands, or anything else without switching windows.

## Working with AI agents

### Agent chat

Open the side panel (Ctrl+p if hidden, then Ctrl+1) and type a message. The agent can read your project files but cannot write directly — every proposed change goes through the Write Gate.

Useful commands you can type in the chat:

| Command | What it does |
|---|---|
| `/model <name>` | Switch Claude model |
| `/compact` | Trim conversation history |
| `/remember <text>` | Store a fact in semantic memory |
| `/attach <path>` | Attach a file to the conversation |
| `/detach <path>` | Remove an attachment |

### The Write Gate

When an agent proposes file changes, the editor opens a diff overlay showing each affected function or block. You review and accept or reject individual hunks:

| Key | Action |
|---|---|
| ]h / [h | Next / previous hunk |
| a / r | Accept / reject current hunk |
| A / R | Accept / reject all hunks |
| f | Finalize — write accepted changes to disk |
| q | Exit review |

Each hunk shows its enclosing AST node (function name, struct, class) so you know exactly what's being changed.

### Swarm mode

For larger tasks, you can coordinate multiple agents working in parallel:

```
/cswarm refactor the authentication module to use JWT tokens
```

The coordinator (Opus) decomposes the task into a dependency graph, assigns each subtask to an agent with a specific file scope and a model tier, and executes them tier by tier. Each agent works in its own git worktree. After all agents finish, branches are merged automatically, with Claude resolving any conflicts.

Model routing is automatic — the coordinator annotates each subtask with a tier and the router selects the model:

| Tier | Model | Used for |
|---|---|---|
| Coordinator | Opus | Planning, decomposition, verification |
| Reasoning | Sonnet | Complex multi-file semantic changes |
| Execution | Haiku | Focused single-file tasks |
| Mechanical | Ollama (local) | Rote/trivial changes (falls back to Haiku) |

Individual work units can override the tier with an explicit `model` field.

The **Swarm Dashboard** (Ctrl+2) shows real-time status: which agents are running, their output, elapsed time, and cost.

You can also define work units manually:

```
/swarm [{"id":"auth","description":"...","scope":{"owned_paths":["src/auth/"]}}]
```

### Semantic memory

Agents can store and retrieve knowledge across sessions. Memory is backed by ONNX embeddings and SQLite:

```
/remember the authentication module uses bcrypt for password hashing
```

Memory namespaces are configured per-project in `.gaviero/settings.json`.

## Headless CLI

Run agent tasks without the editor:

```
gaviero-cli --repo ~/my-project --task "fix all compilation errors" --auto-accept
```

For coordinated multi-agent tasks:

```
gaviero-cli --repo ~/my-project \
  --task "add comprehensive test coverage for the API layer" \
  --coordinated \
  --max-parallel 4
```

In coordinated mode, model selection is automatic — Opus plans the task, then each subtask is routed to the appropriate model tier (see [Swarm mode](#swarm-mode)). The `--model` flag only applies to non-coordinated single-agent runs.

### CLI flags

| Flag | Description |
|---|---|
| `--repo PATH` | Workspace root (default: current directory) |
| `--task TEXT` | Task description (creates one agent) |
| `--work-units JSON` | WorkUnit array for multi-agent tasks |
| `--coordinated` | Use Opus to plan, then tier-routed execution (ignores `--model`) |
| `--auto-accept` | Skip interactive review |
| `--max-parallel N` | Parallel agent limit (default: 1) |
| `--model NAME` | Claude model for single-agent mode (default: sonnet) |
| `--namespace NS` | Memory write namespace |
| `--read-ns NS` | Additional read namespaces (repeatable) |
| `--format text\|json` | Output format |

## Configuration

Settings cascade in priority order:

1. `.gaviero/settings.json` in the project directory
2. `settings` block in a `.gaviero-workspace` file
3. `~/.config/gaviero/settings.json` (user-level)
4. Built-in defaults

### Common settings

```json
{
  "editor": { "tabSize": 4, "insertSpaces": true },
  "files": { "exclude": { "target": true, "node_modules": true } },
  "agent": { "model": "sonnet", "maxTokens": 16384 },
  "memory": { "namespace": "my-project" },
  "panels": { "fileTree": { "width": 25 }, "terminal": { "splitPercent": 30 } }
}
```

Language-specific overrides use bracket syntax: `"[rust]": { "editor.tabSize": 4 }`.

### Workspace files

For multi-folder projects, create a `.gaviero-workspace` file:

```json
{
  "folders": [
    { "path": "/home/user/frontend", "name": "Frontend" },
    { "path": "/home/user/backend", "name": "Backend" }
  ],
  "settings": { "agent": { "model": "sonnet" } }
}
```

### Themes

Color schemes live in `themes/` as TOML files. The default theme is One Dark inspired.

## Architecture at a glance

Gaviero is a Cargo workspace with three crates:

| Crate | Role |
|---|---|
| `gaviero-core` | All logic: write gate, diffs, tree-sitter (16 languages), agent subprocess management, swarm orchestration, git (via git2), semantic memory, terminal PTY |
| `gaviero-tui` | Terminal UI: ratatui + crossterm rendering, panels, input handling |
| `gaviero-cli` | Headless runner: clap argument parsing, stdout observers |

Core never depends on any UI crate. The TUI communicates with core pipelines through observer traits and a single event channel.

For full architectural details, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Requirements

- Rust (2024 edition)
- Linux or POSIX terminal
- Claude API key (for agent features — the editor works fine without one)

## License

Apache License 2.0. See [LICENSE](LICENSE).
