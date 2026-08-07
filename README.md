# Gaviero

## Overview

Gaviero is a terminal editor for collaborating with AI agents on code — file tree, syntax highlighting, git integration, embedded terminal, and an agent chat panel where every proposed change passes through an interactive Write Gate before touching disk. A headless CLI (`gaviero-cli`) runs the same engine in CI; the `.gaviero` DSL composes multi-agent workflows declaratively.

## Installation

Build from source (Rust 2024 edition required):

```bash
cargo build --release
```

Binaries:

| Binary | Crate | Purpose |
|---|---|---|
| `gaviero` | `gaviero-tui` | Interactive TUI editor |
| `gaviero-cli` | `gaviero-cli` | Headless command-line runner |
| `gaviero-mcp-shim` | `gaviero-mcp-shim` | stdio↔socket bridge for subprocess agent MCP access |

```bash
./target/release/gaviero ~/my-project
./target/release/gaviero-cli --task "fix compilation errors" --auto-accept
```

## Usage

### Editor

```bash
gaviero                              # current directory
gaviero /path/to/repo                # single-folder workspace
gaviero /path/to/project.gaviero-workspace   # multi-folder workspace
gaviero --workspace ~/src            # multi-folder workspace built from ~/src
```

**First run.** Opening a folder with no Gaviero configuration starts a short setup wizard before the editor. It asks for an agent profile — *full capabilities* (shell plus the edit tools, auto-approved, allow/deny-listed) or *restricted* (identical minus `Bash`, which is never offered to the agent) — and whether to write the Claude, Codex and Cursor MCP configs. With `--workspace` it also asks which sub-folders join the workspace (git repositories pre-selected) and writes `<dirname>.gaviero-workspace`. Esc on the first screen skips setup and opens the folder with built-in defaults; existing files are never overwritten.

The editor works standalone without AI features. Keybindings, panels, chat commands, and the Write Gate diff review are documented in [crates/gaviero-tui/README.md](crates/gaviero-tui/README.md).

### Headless CLI

```bash
gaviero-cli --repo ~/my-project --task "fix all compilation errors" --auto-accept

gaviero-cli --repo ~/my-project \
  --task "add test coverage for the API layer" \
  --coordinated --max-parallel 4

gaviero-cli --script workflows/review.gaviero --var PLANS=output
```

In coordinated mode, model selection is automatic — the coordinator plans, then each subtask is routed to a tier. The `--model` flag applies only to non-coordinated single-agent runs. See [crates/gaviero-cli/README.md](crates/gaviero-cli/README.md) for the full flag reference.

### Workflow scripts

Define reusable multi-agent workflows in `.gaviero` files and run them with `gaviero-cli --script` or `/run` in the TUI. Shipped templates live under [`crates/gaviero-dsl/examples/`](crates/gaviero-dsl/examples/) (e.g. scientific research consensus). See [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md) for the language reference.

## Examples

**Chat with an agent** (in the TUI agent panel):

```
review src/auth/session.rs for race conditions
/model claude:sonnet
/lite
```

**Ad-hoc multi-agent swarm:**

```
/cswarm refactor the authentication module to use JWT tokens
```

**DSL workflow** (`review.gaviero`):

```gaviero
client sonnet { tier cheap     model "claude:sonnet" effort low  default }
client opus   { tier expensive model "claude:opus"   effort high }

tier cheap     sonnet
tier expensive opus

agent reviewer {
    description "Review the PR and identify issues"
    client opus
    scope { read_only ["src/" "tests/"] }
    prompt "Review {{PROMPT}} and list all bugs and style issues."
}

agent fixer {
    description "Fix all issues found by the reviewer"
    client sonnet
    depends_on [reviewer]
    scope { owned ["src/" "tests/"] impact_scope true }
    prompt "Fix every issue in the reviewer's list."
}

workflow review_and_fix {
    steps [reviewer fixer]
    verify { compile true  clippy true  impact_tests true }
}
```

```bash
gaviero-cli --script review.gaviero --prompt "the auth module"
```

**Skills** — turn-scoped instruction templates under `.gaviero/skills/<name>/SKILL.md`, invoked in chat with `$skill-name`. Use `/skills` to list or `/skills search <query>` for semantic discovery.

## Configuration

Settings cascade (highest priority first):

1. `.gaviero/settings.json` — project-level
2. `.gaviero-workspace` file — multi-folder workspace
3. `~/.config/gaviero/settings.json` — user defaults
4. Built-in defaults

```json
{
  "editor": { "tabSize": 4, "insertSpaces": true, "wordWrap": false },
  "files": { "exclude": { "target": true, "node_modules": true } },
  "agent": { "model": "claude:sonnet", "maxTokens": 16384 },
  "memory": { "namespace": "my-project" },
  "panels": { "fileTree": { "width": 25 }, "terminal": { "splitPercent": 30 } }
}
```

Language-specific overrides use bracket syntax: `"[rust]": { "editor.tabSize": 4 }`.

**Multi-folder workspaces** — create a `.gaviero-workspace` file:

```json
{
  "folders": [
    { "path": "/home/user/frontend", "name": "Frontend" },
    { "path": "/home/user/backend", "name": "Backend" }
  ],
  "settings": { "agent": { "model": "claude:sonnet" } }
}
```

**Model specs** use `provider:model` — bare names are rejected. Providers: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:`.

Per-crate configuration details: [gaviero-tui](crates/gaviero-tui/README.md#configuration), [gaviero-core](crates/gaviero-core/README.md#configuration), [gaviero-cli](crates/gaviero-cli/README.md#configuration).

## API

Gaviero is a Cargo workspace of six crates. Core holds all runtime logic with no UI dependencies; the TUI and CLI are thin frontends.

| Crate | README | Purpose |
|---|---|---|
| **gaviero-core** | [README](crates/gaviero-core/README.md) | Swarm, memory, MCP server, Write Gate, ACP, agent sessions, repo map, git |
| **gaviero-tui** | [README](crates/gaviero-tui/README.md) | Terminal UI: editor, panels, chat, diff review |
| **gaviero-cli** | [README](crates/gaviero-cli/README.md) | Headless runner: tasks, scripts, eval, memory admin |
| **gaviero-dsl** | [README](crates/gaviero-dsl/README.md) | `.gaviero` compiler → `CompiledPlan` |
| **gaviero-mcp-shim** | [README](crates/gaviero-mcp-shim/README.md) | stdio↔Unix-socket / Windows-named-pipe MCP bridge |
| **tree-sitter-gaviero** | [README](crates/tree-sitter-gaviero/README.md) | Tree-sitter grammar for `.gaviero` files |

For module maps, data flow, and inter-crate boundaries, see [ARCHITECTURE.md](ARCHITECTURE.md). For build/test conventions, see [CLAUDE.md](CLAUDE.md).

## Requirements

- **Rust 2024 edition** — for building from source
- **Linux, macOS, or Windows 11+** — terminal environment for the TUI. On Windows: Windows Terminal, **PowerShell 7.2+**, and **Git for Windows** are required (Windows PowerShell 5.1 and cmd are unsupported)
- **Claude API key** (optional) — required only for AI agent features; the editor works standalone

## Getting Help

- Build and test: [CLAUDE.md](CLAUDE.md)
- Architecture: [ARCHITECTURE.md](ARCHITECTURE.md)
- Bug reports and feature requests: open an issue in this repository

## License

Apache License 2.0. See [LICENSE](LICENSE).
