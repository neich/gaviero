# Gaviero

**A terminal editor for working with AI coding agents (Claude Code, Codex, Cursor, DeepSeek, Ollama). Every change an agent makes is reviewed before the conversation continues.**

[![CI](https://github.com/neich/gaviero/actions/workflows/ci.yml/badge.svg)](https://github.com/neich/gaviero/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

<!-- DEMO: put a 20–40 s recording here (asciinema or GIF): ask an agent for a change, reject one file in TURN REVIEW, keep the rest, continue. -->

During a turn, agents work freely: edit tools, shell commands, formatters, deletes, renames. When the turn ends, gaviero compares the workspace with a snapshot taken before your prompt and opens a review. For each file you either keep the change or go back to the pre-prompt version, and the conversation waits until every file is decided. No git required.

## Why gaviero

- **Review what the turn did, not what the tools reported.** Turn capture diffs the files on disk, so a change made by `sed`, `cargo fmt`, a code generator or a stray `rm` is reviewed like any edit-tool change. Sensitive files (`.env`, keys) an agent changed are restored automatically.
- **One editor, many agents.** Claude Code, Codex, Cursor CLI, DeepSeek (API or `dsh`) and Ollama run behind the same chat panel, activity stream and review path. They use your existing logins and subscriptions. Switch with `/model codex:…`.
- **One policy for every provider.** Read rules, bash allow/deny lists, MCP servers and network policy live in `.gaviero/settings.json`. At launch gaviero compiles them into each vendor's own format, and a loopback proxy logs which hosts each agent connects to.
- **Multi-agent workflows as code.** The `.gaviero` DSL declares agents, file scopes, dependencies, loops and verification steps (compile, clippy, impacted tests). Scripts ask for a model *tier* rather than a vendor, so one workflow runs on any provider, from the SWARM panel or headless in CI.
- **Shared code graph and memory.** A built-in MCP server gives every agent the same tools: a ranked repo outline, blast radius, symbol references, and project memory that persists across sessions.
- **A real editor when the agents are off.** Tree-sitter highlighting, file tree, project search, git, merge-conflict navigation, an embedded terminal, multi-folder workspaces and session tabs. *Editor only* mode switches every AI feature off.

## How it compares

- **Running an agent CLI on its own** (Claude Code, Codex CLI, Cursor CLI): each vendor reviews its own edit tools in its own way, and what a shell command changes usually shows up only in `git diff`. Gaviero runs those same CLIs and adds one review covering everything a turn changed, one settings file, and one code-graph/memory layer shared by all of them.
- **IDE agent panels** (Cursor, Zed, VS Code extensions): GUI editors with built-in agents. Gaviero is terminal-native, so it works over SSH and inside tmux, and it treats the vendor CLIs as interchangeable back ends.
- **Agent launchers and multiplexers** (several agents in panes or git worktrees): they start and isolate agents. Gaviero also reviews what each turn changed, shares context between agents, and declares multi-agent runs as scripts.

Gaviero is young: expect rough edges, and please [open an issue](https://github.com/neich/gaviero/issues) when you hit one.

## Quick start

**1. Install.** Prebuilt archives for Linux (x86_64), macOS (Apple Silicon) and Windows (x86_64) are on the [Releases](https://github.com/neich/gaviero/releases) page. Unpack one and put its folder on your `PATH`. Keep the three binaries together, because `gaviero` looks for `gaviero-cli` and `gaviero-mcp-shim` next to itself.

From source (Rust stable, 2024 edition):

```bash
cargo install --locked --git https://github.com/neich/gaviero gaviero-tui gaviero-cli gaviero-mcp-shim
```

or clone the repository and run `cargo build --release` (binaries in `target/release/`).

**2. Have an agent available.** Install and log in to at least one of: [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (`claude`), [Codex CLI](https://github.com/openai/codex) (`codex`), [Cursor CLI](https://docs.cursor.com/en/cli/overview) (`agent`), [Ollama](https://ollama.com) running locally, or a DeepSeek API key (`DEEPSEEK_API_KEY`). The editor also works with none of them.

**3. Open a project.**

```bash
gaviero ~/my-project
```

The first launch shows a setup screen. Pick *Minimal* or *Full settings* to enable the agent panel, or *Editor only* to try the editor alone. Open the agent chat with `Alt+A`, choose a model with `/model claude:sonnet` (any `provider:model`), and ask for a change. When the turn ends, TURN REVIEW lists every file it changed: `a` / `r` keeps or reverts the selected file, and `A` / `R` keeps or reverts the whole turn.

Keybindings, panels and chat commands: [crates/gaviero-tui/README.md](crates/gaviero-tui/README.md).

| Binary | Crate | Purpose |
|---|---|---|
| `gaviero` | `gaviero-tui` | Interactive TUI editor |
| `gaviero-cli` | `gaviero-cli` | Headless runner: tasks, swarms, workflow scripts |
| `gaviero-mcp-shim` | `gaviero-mcp-shim` | stdio↔socket bridge that gives subprocess agents gaviero's MCP tools |

## Usage

### Editor

```bash
gaviero                              # current directory
gaviero /path/to/repo                # single-folder workspace
gaviero /path/to/project.gaviero-workspace   # multi-folder workspace
gaviero --workspace ~/src            # multi-folder workspace built from ~/src
gaviero --folder ~/src               # plain folder, even if ~/src holds a workspace file
```

A plain `gaviero <dir>` on a directory that holds a `.gaviero-workspace` file asks whether to open that workspace instead (memory, code graph and member settings differ between the two); `--folder` skips the question.

**Several sessions in one terminal.** `gaviero --sessions [path]` starts a session host: every session is its own `gaviero` process, shown under a tab bar. Alt+Left / Alt+Right switch sessions, Alt+Shift+Left / Alt+Shift+Right reorder them, Alt+N opens another folder or workspace, Alt+Q (or the `x` on a tab) closes the tab and ends its gaviero, Ctrl+Q quits the host and reopens every tab at the next start; both ask first when an agent, a swarm or a shell command is running or files are unsaved. Two letter badges on each tab, `A` and `S`, show its agent (running, waiting for you, finished) and its shells (a command running, one waiting for your input, one finished); a tab keeps them when it is made current, and a finished badge clears once you leave the tab. Without a path it reopens the sessions open when the host last ended. Only one host runs at a time; a second `gaviero --sessions` warns and refuses to start. After a deploy the host restarts itself and its tabs onto the new build once every tab is idle. See [crates/gaviero-tui/README.md](crates/gaviero-tui/README.md#usage).

**First run.** Opening a folder that has no `.gaviero/settings.json` shows a setup screen before the editor, at every launch until the folder has one. It offers three choices:

1. *Editor only* (preselected; Esc picks it too) — nothing is written to the folder. You get the explorer, find, the editor and the terminal; there is no side panel, agent, memory or code graph. Settings come from the global `$GAVIERO_ROOT/.gaviero/settings.json` (or the built-in defaults).
2. *Minimal settings* — writes `.gaviero/settings.json` containing `{}`, so every setting is inherited from the global file.
3. *Full settings* — writes `.gaviero/settings.json` as an exact copy of the global file as it is at that moment. The copy includes any `remote.relay.secret`, so the screen warns you not to commit it. Without a global file it writes gaviero's built-in full settings instead (every agent tool auto-approved, a bash allow/deny list, default excludes and layouts) and names the path it looked for.

Opening a `.gaviero-workspace` file that has no `.gaviero/settings.json` beside it shows the same screen; choices 2 and 3 write that workspace-level file, which every member folder inherits (a member's own `.gaviero/settings.json` still overrides it). With `--workspace` on a directory that has no workspace file yet, the screen first asks which sub-folders join (git repositories pre-selected) and writes `<dirname>.gaviero-workspace` — editor-only then writes only that file. Existing files are never overwritten.

The editor works standalone without AI features. Keybindings, panels, chat commands, and the turn review are documented in [crates/gaviero-tui/README.md](crates/gaviero-tui/README.md).

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

Define reusable multi-agent workflows in `.gaviero` files and run them from the TUI's SWARM panel (`Alt+W`) or headless with `gaviero-cli --script`. Agent chat does not run scripts. Shipped templates live under [`crates/gaviero-dsl/examples/`](crates/gaviero-dsl/examples/) (e.g. scientific research consensus). See [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md) for the language reference.

## Examples

**Chat with an agent** (in the TUI agent panel):

```
review src/auth/session.rs for race conditions
/model claude:sonnet
/lite
```

**Inspect a turn** (history panel, `Alt+H`): the full prompt, every tool call with arguments and result, every MCP request/response, the memory call and its injected block, and token totals (`~` estimates, `exact` provider usage). `c` copies the focused section's records as NDJSON; `y` copies it as plain text — on PROMPT, the prompt verbatim, ready to paste or send again. Headless: `gaviero-cli --history`.

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

**Skills** — turn-scoped instruction templates under `.gaviero/skills/<name>/SKILL.md`, invoked in chat with `$skill-name`. They are found in each folder's `.gaviero/skills/`, in the workspace's — the `.gaviero/skills/` beside the `.gaviero-workspace` file, also for a one-folder workspace or a member folder opened on its own — and in the global `$GAVIERO_ROOT/.gaviero/skills/` (folder beats workspace beats global). Adding or removing a skill is picked up while gaviero runs, even when the `skills/` directory is created after launch. Use `/skills` to list or `/skills search <query>` for semantic discovery; `/skills` also lists every folder it scanned and each skill it could not load, with the reason (a missing `description`, a frontmatter `name` that differs from the folder name, a folder name outside `A-Za-z0-9_-`). `SKILL.md` may be any letter case, start with a BOM, and use YAML block values (`description: >`). Optional `skills.extraRoots` (array of paths, default `[]`, `~` expanded) adds foreign roots such as `~/.claude/skills` or `~/.codex/skills`; a missing configured root is a chat warning, not a silent skip.

## Configuration

Settings are JSON, loaded in three layers. Each later layer overrides the one before it, key by key:

1. **User**: `$GAVIERO_ROOT/.gaviero/settings.json`, or `~/.gaviero/settings.json` when `GAVIERO_ROOT` is unset.
2. **Workspace**: the `.gaviero/settings.json` beside a `*.gaviero-workspace` file.
3. **Repo**: each workspace folder's own `.gaviero/settings.json`.

`/settings`, `/settings workspace` and `/settings repo` open a layer in the editor and highlight keys that another layer overrides.

```json
{
  "editor": { "tabSize": 4, "insertSpaces": true, "wordWrap": false },
  "files": { "exclude": { "target": true, "node_modules": true } },
  "agent": { "model": "claude:sonnet", "maxTokens": 16384 },
  "memory": { "namespace": "my-project" },
  "panels": { "fileTree": { "width": 25 }, "terminal": { "splitPercent": 30 } }
}
```

**Model specs** use `provider:model`; bare names are rejected. Providers: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:`, `dsh:`.

The full reference is in **[docs/configuration.md](docs/configuration.md)**. It covers layer merging and `key+` extension, multi-folder workspaces, `GAVIERO.md` project instructions, agent permissions and bash allow/deny lists, protected files per provider, and outbound-traffic logging.

## Crates

Gaviero is a Cargo workspace. Core holds all runtime logic with no UI dependencies; the TUI and CLI are thin frontends.

| Crate | README | Purpose |
|---|---|---|
| **gaviero-core** | [README](crates/gaviero-core/README.md) | Agent sessions, turn capture, Write Gate, swarm, memory, MCP server, repo map, git |
| **gaviero-tui** | [README](crates/gaviero-tui/README.md) | Terminal UI: editor, panels, chat, turn review |
| **gaviero-cli** | [README](crates/gaviero-cli/README.md) | Headless runner: tasks, scripts, eval, memory admin |
| **gaviero-dsl** | [README](crates/gaviero-dsl/README.md) | `.gaviero` compiler → `CompiledPlan` |
| **gaviero-mcp-shim** | [README](crates/gaviero-mcp-shim/README.md) | stdio↔Unix-socket / Windows-named-pipe MCP bridge |
| **tree-sitter-gaviero** | [README](crates/tree-sitter-gaviero/README.md) | Tree-sitter grammar for `.gaviero` files |

For module maps, data flow and crate boundaries, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Requirements

- **Linux, macOS, or Windows 11+.** On Windows: Windows Terminal, **PowerShell 7.2+** and **Git for Windows** are required (Windows PowerShell 5.1 and cmd are unsupported).
- **Rust stable (2024 edition)**, only when building from source.
- **For AI features**, at least one agent: Claude Code, Codex CLI, Cursor CLI, Ollama, or a DeepSeek API key. The editor works standalone.

## Contributing

Bug reports, ideas and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for building, testing and the project's ground rules.

## License

Apache License 2.0. See [LICENSE](LICENSE).
