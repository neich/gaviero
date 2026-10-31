# Gaviero

## Overview

Gaviero is a terminal editor for collaborating with AI agents on code — file tree, syntax highlighting, git integration, embedded terminal, and an agent chat panel where agents edit freely while they work and every change a turn made — edit tools, shell commands, formatters — is reviewed afterwards: keep it, or go back to the pre-prompt version per file or hunk, before the conversation continues. The capture needs no git. A headless CLI (`gaviero-cli`) runs the same engine in CI; the `.gaviero` DSL composes multi-agent workflows declaratively.

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

Define reusable multi-agent workflows in `.gaviero` files and run them with `gaviero-cli --script` or `/run` in the TUI. Shipped templates live under [`crates/gaviero-dsl/examples/`](crates/gaviero-dsl/examples/) (e.g. scientific research consensus). See [crates/gaviero-dsl/README.md](crates/gaviero-dsl/README.md) for the language reference.

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

**Skills** — turn-scoped instruction templates under `.gaviero/skills/<name>/SKILL.md`, invoked in chat with `$skill-name`. Use `/skills` to list or `/skills search <query>` for semantic discovery. Optional `skills.extraRoots` (array of paths, default `[]`, `~` expanded) adds foreign roots such as `~/.claude/skills` or `~/.codex/skills`; a missing configured root is a chat warning, not a silent skip.

## Configuration

Settings cascade (highest priority first):

1. `.gaviero/settings.json` — project-level
2. `.gaviero-workspace` file — multi-folder workspace
3. `~/.gaviero/settings.json` — user defaults (same home-dir path on Windows and Linux). If that file is missing, the previous location is still read (`~/.config/gaviero/settings.json` on Unix, `%APPDATA%\gaviero\settings.json` on Windows). Set `GAVIERO_ROOT` to share one user directory between gaviero instances: `settings.json`, `secrets.toml` and `skills/` are then read from `$GAVIERO_ROOT/.gaviero` instead of `~/.gaviero`. The workspace/repo `.gaviero/` (beside the `.gaviero-workspace` file, or in the opened folder) is always loaded first and overrides it. Per-machine state (`remote/`, `dsh/`) stays in `~/.gaviero`.
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

**Agent permissions** are defined once under `agent` and pushed to every provider. Do not author shell rules in `.claude/settings.json`, `.cursor/cli.json` or `.codex/` — gaviero regenerates those from these keys on every launch and inside every swarm worktree (hand-written entries survive, but they never reach the other providers).

**Project instructions** come from `GAVIERO.md`, one file for every provider. When a repository (or an enclosing `*.gaviero-workspace` folder) has one, gaviero switches off the provider's own `CLAUDE.md` / `AGENTS.md` discovery — Claude `claudeMdExcludes`, Codex `project_doc_max_bytes=0`, dsh's `agent-instructions` row, and a managed `.cursorignore` block for Cursor — and injects `GAVIERO.md` instead. Files are read from the repository root down to the workspace folder, so the deepest file wins; `$GAVIERO_ROOT/.gaviero/GAVIERO.md` is the user-wide baseline and never hides a project file on its own. Nested `GAVIERO.md` files are listed for the agent to read when it works in that folder, the way nested `CLAUDE.md` files are. There is no opt-out: a `GAVIERO.md` in the tree is always in effect. Claude's own `/compact` is not special — gaviero re-reads the file on the next turn, so edits apply after a compaction or reset.

| Key | Meaning | Default |
|---|---|---|
| `agent.availableTools` | Hard tool surface. Claude `--tools`; Cursor `Shell(*)` / `Write(**)` deny rules when `Bash` / write tools are absent; Codex chat declines commands and file changes that are off the surface; tool registry of the in-process agent. | `Read, Glob, Grep, Write, Edit, MultiEdit` |
| `agent.approvedTools` | Subset that never prompts. Claude `--allowedTools`; Cursor gets `--force` only when this contains `Bash`; Codex chat and the in-process agent skip the prompt. | `Read, Glob, Grep` |
| `agent.permissions.bash.allowlist` | Command prefixes that run without a prompt even when `Bash` is not approved. Claude `Bash(prefix:*)` and Cursor `Shell(prefix*)` allow rules; enforced directly by Codex chat and the in-process agent. Set `[]` to disable. | read-only commands plus `cargo check / test / build / clippy` |
| `agent.permissions.bash.denylist` | Command token sequences that never run, whatever else is approved. Claude and Cursor deny rules; Codex `.codex/rules/gaviero.rules` (`forbidden`, enforced even under the swarm bypass flag); enforced directly by Codex chat and the in-process agent. | none (a built-in list blocks `sudo`, `rm -rf`, `curl … \| sh`) |
| `agent.permissions.bash.timeoutSecs` / `outputCapBytes` | Limits for one in-process shell command. | `120` / `30720` |
| `agent.permissions.sensitivePaths.allow` | Path globs exempted from the sensitive-file list (`.env*`, keys, `*.pem`, `.npmrc`, `.docker/config.json`, …). Never exempts the hard floor: agent credentials (`~/.claude/.credentials.json`, `~/.codex/auth.json`, `~/.gaviero/remote`, `~/.gaviero/secrets.toml`) and gaviero's own state (`.gaviero/history`, `turns`, `egress`, `remote`, `mcp-http-token`, `memory.db*`, …). | none |
| `agent.permissions.read.deny` | Extra workspace-relative globs agents may never read. | none |
| `agent.permissions.read.blockOutsideWorkspace` | Refuse agent file reads outside the workspace folders (Claude `blockReadsOutsideWorkingDirectories`, which also covers Bash; commands Claude cannot analyse, such as `node -e`, prompt instead). | `true` |
| `agent.network.mode` | `off` · `log` · `ask` · `enforce`. Every agent CLI and gaviero's own HTTP clients go through a loopback CONNECT proxy (no TLS decryption) that records host:port per provider in `.gaviero/egress/connections.ndjson`. `log` warns in chat on hosts not in `agent.network.allow`, `ask` holds the connection for your decision, and `enforce` refuses it. `gaviero-cli` treats `ask` as `enforce`. | `log` |
| `agent.network.allow` | Host patterns that never warn (`host` or `*.suffix`). | `*.anthropic.com`, `chatgpt.com`, `*.chatgpt.com`, `api.openai.com`, `*.openai.com`, `*.cursor.sh`, `api.deepseek.com`, `context7.com`, `*.context7.com` |
| `agent.network.askTimeoutSecs` | How long an `ask` waits before denying. | `60` |
| `agent.egress.captureClaudeBodies` | Have Claude write each raw API request body (`OTEL_LOG_RAW_API_BODIES`). After the turn, gaviero scans the bodies for secrets, warns in chat, logs one row per request (size, SHA-256, redacted findings) to `.gaviero/egress/claude-requests.ndjson`, and deletes them. | `true` |

Matching: an allowlist entry is a prefix on a word boundary, and every segment of a compound command (`&&`, `||`, `;`, `|`) must match one — `cargo test && curl x` is not cleared by `cargo test`. A denylist entry matches as a token sequence anywhere in the command, last token as a prefix — `git push --force` catches `cd x && git push --force-with-lease`; `gh` catches `gh pr create` but not `echo high`. Claude and Cursor only support prefix rules natively, and Codex only token prefixes, so a denied sequence buried mid-segment is caught by gaviero's own gate (Codex chat, DeepSeek) but not by Claude or Cursor.

What happens to a command that is available but neither approved nor allow-listed: Claude, Codex chat and DeepSeek prompt you; headless Cursor auto-rejects it. Swarm agents always run auto-approved, so only the denylist applies there.

### Protected files and outbound traffic

`.gaviero/settings.json` decides which files agents may read and where they may connect. Gaviero compiles that policy into each provider's own form at every launch; never edit the provider files by hand.

**Choose which files are off-limits:**

```json
{
  "agent": {
    "permissions": {
      "read": {
        "deny": ["private/**", "config/prod.yaml"],
        "blockOutsideWorkspace": true
      },
      "sensitivePaths": {
        "allow": ["tests/fixtures/**"]
      }
    }
  }
}
```

| Category | Files | Can you exempt it? |
|---|---|---|
| Your additions | Globs listed in `agent.permissions.read.deny`, relative to the folder | You remove the entry |
| Built-in sensitive files | `.env` and `.env.*` (templates such as `.env.example` stay readable), `id_rsa`, `id_ed25519`, `id_dsa`, `id_ecdsa`, `.ssh/`, `.aws/`, `credentials`, `.netrc`, `secrets.{toml,yaml,json}`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks`, `*.keystore`, `.npmrc`, `.pypirc`, `.git-credentials`, `.pgpass`, `.docker/config.json`, `.kube/config` | Yes, with `agent.permissions.sensitivePaths.allow` |
| Hard floor | `~/.claude/.credentials.json`, `~/.codex/auth.json`, `~/.gaviero/remote`, `~/.gaviero/secrets.toml`, and gaviero's own state: `.gaviero/history`, `turns`, `egress`, `remote`, `mcp-http-token`, `secrets.toml`, `mcp_calls.ndjson`, `memory.db*`, `code_graph.db*` | Never |
| Outside the workspace | Anything outside the workspace folders, while `agent.permissions.read.blockOutsideWorkspace` is `true` (the default) | Set it to `false` |

`.gaviero/settings.json`, `.gaviero/skills/` and `.gaviero/tmp/` stay readable; spilled prompts live in `.gaviero/tmp/`.

**How each provider gets the policy:**

| Provider | How the policy is applied |
|---|---|
| Claude | `--setting-sources user,project` plus a `--settings` layer of `Read(...)` deny rules (they cover the Read tool, Grep and Bash `cat`) and the outside-workspace block. A `PostToolUse` Bash hook (`gaviero-cli --hook claude:post-tool-use`) withholds output lines from protected files, e.g. `grep -r` hits, and redacts secret-looking values. A startup chat message lists any `~/.claude/settings.json` allow rules that would skip gaviero's prompts. |
| Cursor | `.cursor/hooks.json`, written in each workspace folder at startup (`beforeReadFile`, `beforeShellExecution` → `gaviero-cli --hook cursor:…`). Cursor's `cli.json` `Read()` denies do not stop reads. |
| Codex (chat) | A deny-read permission profile in `thread/start`, running under the MXC sandbox on Windows. If Codex cannot enforce it, the session fails closed with an explanation. Shell environment variables named `*KEY*`, `*SECRET*` or `*TOKEN*` are filtered out. |
| `deepseek:` | Enforced in-process, Bash file arguments included. |

Gaviero itself refuses to inline a protected `@`-mention or attachment; a chat message says which rule matched. It also redacts secret-looking values from injected memory, memory-extractor transcripts and the history log.

**Watch outbound traffic:**

```json
{
  "agent": {
    "network": {
      "mode": "log",
      "allow": ["*.anthropic.com", "chatgpt.com", "*.cursor.sh"],
      "askTimeoutSecs": 60
    },
    "egress": { "captureClaudeBodies": true }
  }
}
```

`mode` can be:
- `off`: no proxy.
- `log` (default): warn in chat the first time a provider reaches a host not on the allowlist.
- `ask`: hold the connection until you answer in the permission overlay. `gaviero-cli` has no prompt, so it refuses instead.
- `enforce`: refuse hosts not on the allowlist.

**Logs**, in `<folder>/.gaviero/egress/` (agents cannot read this directory):

| File | What it records |
|---|---|
| `connections.ndjson` | One row per connection: time, provider (`claude`, `codex`, `agent` for Cursor, `deepseek`, `context7`, …), host, port, decision, bytes up and down, duration. Destinations only: the proxy does not decrypt TLS. |
| `claude-requests.ndjson` | One row per Claude API request: session, model, size, SHA-256, and any secret-looking values found (redacted excerpts only). The raw bodies are deleted after the scan. |
| `hooks.ndjson` | Every Claude and Cursor hook decision: the path or command, and whether it was allowed, denied or redacted, with the reason. |

Only Claude's request contents are captured; for the other providers the log shows which hosts they reached, not what they sent.

**Known gaps:**
- Cursor recursive searches (`grep -r`, `rg`) can surface protected content, since Cursor offers no shell-output redaction.
- Cursor reads `.cursor/hooks.json` from the enclosing git repository's root, so a workspace folder nested inside another repository is unprotected unless that root carries the hooks too.
- Claude `sensitivePaths.allow` exemptions do not reach sibling workspace folders.
- Codex ignores `sensitivePaths.allow` exemptions, and its swarm `codex exec` path has no deny profile.
- The proxy observes only clients that honour `HTTPS_PROXY`.

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
