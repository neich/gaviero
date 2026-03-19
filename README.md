# Gaviero

A terminal code editor for AI agent orchestration. Gaviero lets you run swarms of Claude AI agents to perform complex coding tasks — with full human oversight through an interactive diff review workflow.

Built entirely in Rust.

## Features

- **Interactive TUI editor** with syntax highlighting (14 languages), undo/redo, tabs, search, and configurable layouts
- **Write Gate** — every AI-proposed change goes through an approval pipeline where you accept or reject individual hunks before anything touches disk
- **Agent chat** — converse with a Claude agent directly in the side panel; it can read your project files but only writes through the Write Gate
- **Swarm orchestration** — coordinate multiple parallel agents with dependency management, isolated git worktrees, and automatic conflict resolution
- **Embedded terminal** — full shell inside the editor with environment isolation
- **File scoping** — agents can only access or modify the paths you explicitly permit
- **Semantic memory** — agents build and search a shared knowledge base using ONNX embeddings and SQLite
- **Git integration** — staging, commits, branch switching, and merge conflict resolution without leaving the editor
- **Headless CLI** — run swarms from scripts or CI without the TUI

## Requirements

- Rust 2024 edition
- Linux or POSIX-compliant terminal
- A Claude API key (for agent features)

## Installation

```
cargo build --release
```

This produces two binaries in `target/release/`:

| Binary | Purpose |
|---|---|
| `gaviero` | TUI editor |
| `gaviero-cli` | Headless swarm runner |

## Getting started

Open a project directory:

```
gaviero ~/my-project
```

Or a multi-folder workspace:

```
gaviero ~/my-project/.gaviero-workspace
```

The editor opens with a file tree on the left, the code editor in the center, and a collapsible side panel on the right for agent chat, swarm dashboard, or git operations.

## Editor keybindings

### General

| Key | Action |
|---|---|
| Ctrl+q | Quit |
| Ctrl+s | Save |
| Ctrl+z / Ctrl+y | Undo / Redo |
| Ctrl+c / Ctrl+x / Ctrl+v | Copy / Cut / Paste |
| Ctrl+b | Toggle file tree |
| Ctrl+p | Toggle side panel |
| F4 | Toggle terminal |
| Ctrl+t | New tab |
| Ctrl+w | Close tab |
| Alt+[ / Alt+] | Cycle tabs |
| Ctrl+\ | Cycle focus between panels |
| Ctrl+arrow keys | Move focus directionally |
| Ctrl+0 to Ctrl+9 | Layout presets |
| F3 | Search workspace |
| F2 | Rename |
| F5 | Format |

### Editing

| Key | Action |
|---|---|
| Ctrl+k | Delete line |
| Ctrl+d | Duplicate line |
| Alt+Up / Alt+Down | Move line up / down |
| Ctrl+e | Go to line end |
| Ctrl+h | Delete word backward |
| Ctrl+Delete | Delete to line end |

### Side panel modes

| Key | Panel |
|---|---|
| Ctrl+1 | Agent Chat |
| Ctrl+2 | Swarm Dashboard |
| Ctrl+3 | Git |

### Diff review (when reviewing agent proposals)

| Key | Action |
|---|---|
| ]h / [h | Next / previous hunk |
| a / r | Accept / reject current hunk |
| A / R | Accept / reject all hunks |
| f | Finalize changes |
| q | Exit review |

## Headless CLI

Run agent tasks without the TUI:

```
gaviero-cli --repo ~/my-project \
  --task "fix all compilation errors in src/" \
  --max-parallel 4 \
  --model sonnet \
  --auto-accept \
  --format json
```

### CLI options

| Flag | Description |
|---|---|
| `--repo PATH` | Workspace root (default: current directory) |
| `--task TEXT` | Single task description |
| `--work-units JSON` | Array of WorkUnit definitions for multi-agent tasks |
| `--auto-accept` | Accept all scope-valid changes without review |
| `--max-parallel N` | Number of parallel agents (default: 1) |
| `--model NAME` | Claude model to use (default: sonnet) |
| `--namespace NS` | Memory write namespace |
| `--read-ns NS` | Additional read namespaces (repeatable) |
| `--format text\|json` | Output format (default: text) |

## Configuration

Settings are resolved in cascade order — first match wins:

1. `.gaviero/settings.json` in the project directory
2. `settings` block inside a `.gaviero-workspace` file
3. `~/.config/gaviero/settings.json` for user-level defaults
4. Built-in defaults

### Key settings

| Setting | Description |
|---|---|
| `editor.tabSize` | Tab width (default: 4) |
| `editor.insertSpaces` | Use spaces instead of tabs |
| `editor.formatOnSave` | Auto-format on save |
| `files.exclude` | Glob patterns for files to hide |
| `agent.model` | Claude model name |
| `agent.effort` | Agent effort level |
| `agent.maxTokens` | Max tokens per agent response |
| `memory.namespace` | Memory namespace for this project |
| `memory.readNamespaces` | Additional namespaces agents can search |
| `panels.fileTree.width` | File tree panel width |
| `panels.terminal.splitPercent` | Terminal split ratio |

Language-specific overrides are supported using bracket syntax (e.g. `[rust]`, `[javascript]`).

### Workspace files

For multi-folder projects, create a `.gaviero-workspace` file listing folders and shared settings. Pass it as the argument to `gaviero`.

### Themes

Color schemes are defined in TOML files under `themes/`. The default theme is one-dark inspired. Theme files map syntax highlight groups (keyword, string, comment, etc.) to colors and attributes.

## How the Write Gate works

The Write Gate is the core safety mechanism. When an agent proposes a file change:

1. The proposal is queued with a structural diff (showing which functions or blocks are affected)
2. The editor opens a diff overlay where you navigate individual hunks
3. You accept or reject each hunk independently
4. Only accepted changes are written to disk

In swarm mode, changes auto-accept during execution but you review the aggregate result after all agents finish and branches are merged.

## Swarm orchestration

Swarms coordinate multiple agents working on related tasks:

1. **Validate** — check that no two agents claim overlapping file ownership
2. **Execute** — run agents tier-by-tier (parallel within a tier, sequential between tiers), each in its own git worktree
3. **Merge** — bring all branches together with automatic conflict resolution

Agents communicate through a message bus and share context via the semantic memory system.

## Supported languages

Syntax highlighting and structural analysis: Rust, Python, JavaScript, TypeScript, Java, C, C++, HTML, CSS, JSON, TOML, Bash, LaTeX, YAML.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
