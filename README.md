# Gaviero

A terminal editor for collaborating with AI agents on code. Gaviero provides a full development environment — file tree, syntax highlighting, git integration, embedded terminal — alongside a chat panel where you work with Claude agents that can read and modify your code. Every change an agent proposes passes through an interactive review gate before touching disk.

For automation and CI, there's a headless CLI (`gaviero-cli`). For complex multi-agent workflows, the Gaviero DSL lets you compose agents, scopes, verification, and iteration strategies declaratively.

## Installation

Build from source (Rust 2024 edition required):

```bash
cargo build --release
./target/release/gaviero ~/my-project
```

Or to see all binaries:
- `gaviero` — interactive TUI editor
- `gaviero-cli` — headless command-line runner

For full architecture and module details, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Editor Usage

Gaviero works as a standalone editor even without AI features. Use it for everyday coding with full syntax highlighting, git integration, and terminal access.

### Keyboard Navigation

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

## AI Agent Features

### Chat with Agents

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

### Multi-Agent Coordination

For complex tasks, coordinate multiple agents working in parallel:

```
/cswarm refactor the authentication module to use JWT tokens
```

The coordinator (Opus) decomposes the task into a dependency graph, assigns each subtask to an agent with a specific file scope and a model tier, and executes them tier by tier. Each agent works in its own git worktree. After all agents finish, branches are merged automatically, with Claude resolving any conflicts.

Model routing is automatic — the coordinator annotates each subtask with a tier and the router selects the model:

| Tier | Model | Used for |
|---|---|---|
| Coordinator | Opus | Planning, decomposition, verification |
| Expensive | Sonnet | Complex multi-file semantic changes |
| Cheap | Haiku | Focused single-file tasks |
| Mechanical | Ollama (local) | Rote/trivial changes (falls back to Haiku) |

Individual work units can override the tier with an explicit `model` field.

The **Swarm Dashboard** (Ctrl+2) shows real-time status: which agents are running, their output, elapsed time, and cost.

You can also define work units manually:

```
/swarm [{"id":"auth","description":"...","scope":{"owned_paths":["src/auth/"]}}]
```

### Semantic memory

Agents can store and retrieve knowledge across sessions. Memory is backed by ONNX embeddings and SQLite, organized in a five-level scope hierarchy:

```
global        personal cross-workspace knowledge
  └─ workspace    business-level project (.gaviero-workspace)
       └─ repo        single git repository
            └─ module     crate / package / subdirectory
                 └─ run        single swarm execution (consolidated upward on completion)
```

Searches cascade from the narrowest scope outward, stopping early when confidence is high (RRF hybrid vector + keyword search). This means a module-level result beats a global one when it's more relevant.

Store context from the chat panel:
```
/remember the authentication module uses bcrypt for password hashing
```

Memory databases are stored at `<workspace>/.gaviero/memory.db` (workspace-local) and `~/.config/gaviero/memory.db` (global). Configuration is in `.gaviero/settings.json`.

## Headless CLI Usage

Run agent tasks from the command line or CI pipelines:

```bash
gaviero-cli --repo ~/my-project --task "fix all compilation errors" --auto-accept
```

For coordinated multi-agent tasks:

```bash
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
| `--script FILE` | Compile and execute a `.gaviero` workflow file |
| `--work-units JSON` | WorkUnit array for multi-agent tasks |
| `--coordinated` | Use Opus to plan, then tier-routed execution (ignores `--model`) |
| `--output PATH` | Output path for generated plan file (`--coordinated` only) |
| `--auto-accept` | Skip interactive review |
| `--max-parallel N` | Parallel agent limit (default: 1) |
| `--model NAME` | Claude model for single-agent mode (default: sonnet) |
| `--namespace NS` | Memory write namespace |
| `--read-ns NS` | Additional read namespaces (repeatable) |
| `--format text\|json` | Output format |
| `--max-retries N` | Inner validation-feedback retries (default: 5) |
| `--attempts N` | Independent attempts for BestOfN strategy (default: 1) |
| `--test-first` | Generate failing tests before editing (TDD) |
| `--no-iterate` | Single pass only — disables retry loop |
| `--resume` | Skip already-completed agents from a prior run |
| `--trace FILE` | Write DEBUG-level JSON trace log |

## Workflow Scripts (DSL)

Define reusable multi-agent workflows in `.gaviero` files. The Gaviero DSL compiles declarative workflows into execution plans run by the swarm engine. Learn more in [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md).

```gaviero
client sonnet { tier cheap  model "claude-sonnet-4-6" }
client opus   { tier expensive model "claude-opus-4-7" }

agent reviewer {
    description "Review the PR and identify issues"
    client opus
    scope { read_only ["src/" "tests/"] }
    prompt "Review the code changes and list all bugs and style issues."
}

agent fixer {
    description "Fix all issues found by the reviewer"
    client sonnet
    depends_on [reviewer]
    scope {
        owned     ["src/" "tests/"]
        impact_scope true     // auto-expand read_only with blast-radius files
    }
    context {
        callers_of ["src/auth/session.rs"]   // include callers in context
        tests_for  ["src/auth/"]             // include related test files
        depth      2
    }
    prompt "Fix every issue in the reviewer's list."
    max_retries 3
}

workflow review_and_fix {
    steps [reviewer fixer]
    verify {
        compile true
        clippy  true
        impact_tests true   // run only tests affected by modified files
    }
}
```

Run with `gaviero-cli --script review.gaviero` or `/run review.gaviero` in the TUI.

See [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md) for the full language reference.

## Configuration

Settings cascade through these priority levels (highest to lowest):

1. `.gaviero/settings.json` — project-level workspace configuration
2. `.gaviero-workspace` file — multi-folder workspace configuration
3. `~/.config/gaviero/settings.json` — user-level defaults
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

## Architecture

Gaviero is a Cargo workspace of five crates:

| Crate | Purpose |
|---|---|
| **gaviero-core** | All runtime logic: write gate, diffs, tree-sitter (16 languages), agent orchestration, swarm pipeline, context ranking, semantic memory, git/worktrees, terminal PTY |
| **gaviero-tui** | Terminal UI: event loop, panels, editor, chat, diff review, session restore |
| **gaviero-cli** | Headless CLI: task argument parsing, observer wiring |
| **gaviero-dsl** | Compiler for `.gaviero` workflow scripts → execution plans |
| **tree-sitter-gaviero** | Tree-sitter grammar for `.gaviero` files |

Core is the source of truth — it has no UI dependencies. The TUI and CLI both delegate all logic to core through public APIs.

**Crate-specific README files:**
- [crates/gaviero-core/README.md](crates/gaviero-core/README.md) — API entry points, subsystems, design
- [crates/gaviero-tui/README.md](crates/gaviero-tui/README.md) — editor usage, panels, commands
- [crates/gaviero-cli/README.md](crates/gaviero-cli/README.md) — CLI modes, flags, examples
- [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md) — language syntax, examples, compilation

For complete architectural details including data flow, module maps, and inter-crate boundaries, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Requirements

- **Rust 2024 edition** — for building from source
- **Linux or POSIX terminal** — terminal environment for the TUI
- **Claude API key** (optional) — required only for AI agent features; the editor works standalone

## Getting Help

Detailed documentation and development instructions:
- Build and test: see [CLAUDE.md](CLAUDE.md)
- Module structure and subsystems: see [ARCHITECTURE.md](ARCHITECTURE.md)
- For feature requests or bug reports: https://github.com/anthropics/claude-code/issues

## License

Apache License 2.0. See [LICENSE](LICENSE).
