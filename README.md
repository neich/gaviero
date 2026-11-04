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

**Skills** — turn-scoped instruction templates under `.gaviero/skills/<name>/SKILL.md`, invoked in chat with `$skill-name`. They are found in each folder's `.gaviero/skills/`, in the workspace's — the `.gaviero/skills/` beside the `.gaviero-workspace` file, also for a one-folder workspace or a member folder opened on its own — and in the global `$GAVIERO_ROOT/.gaviero/skills/` (folder beats workspace beats global). Adding or removing a skill is picked up while gaviero runs, even when the `skills/` directory is created after launch. Use `/skills` to list or `/skills search <query>` for semantic discovery. Optional `skills.extraRoots` (array of paths, default `[]`, `~` expanded) adds foreign roots such as `~/.claude/skills` or `~/.codex/skills`; a missing configured root is a chat warning, not a silent skip.

## Configuration

Settings are loaded in three layers, and each later layer overrides the one before it, key by key:

1. **User** — `$GAVIERO_ROOT/.gaviero/settings.json`, or `~/.gaviero/settings.json` when `GAVIERO_ROOT` is unset (same home-dir path on Windows and Linux). `GAVIERO_ROOT` shares one user directory between gaviero instances: `settings.json`, `secrets.toml` and `skills/` are read from there. Per-machine state (`remote/`, `dsh/`) stays in `~/.gaviero`. The old `~/.config/gaviero/settings.json` / `%APPDATA%\gaviero\settings.json` is not read.
2. **Workspace** — the `.gaviero/settings.json` beside the `*.gaviero-workspace` file. In a single-folder workspace this is the folder's own file, so layers 2 and 3 are the same file.
3. **Repo** — each workspace folder's own `.gaviero/settings.json`, for that folder.

Built-in defaults apply to keys no layer sets. Objects merge key by key; any other value, lists included, is replaced whole by the more specific layer. To add to an inherited value instead of replacing it, write the key with a trailing `+` (`"mcp.extraServers+": [...]`, or nested `"mcp": { "extraServers+": [...] }`): list entries are appended (duplicates skipped, and an entry whose `"name"` matches an inherited one replaces it), object keys are merged in, and with no broader layer setting the key the built-in default is extended (`"agent.network.allow+": ["example.com"]`). A plain key still replaces everything above it, and a layer may set both (`key` first, then `key+`). Relative `agent.permissions.read.extraFolders` entries resolve against the layer that wrote them. Settings embedded in the `.gaviero-workspace` file itself are not read. Settings gaviero writes for you (such as the Codex trust answer) go to the workspace layer, so a repo file that sets the same key still wins for its folder. Agent policy (`agent.permissions.*`, `agent.network.*`, tools, MCP servers) follows the same order, so a repo's `.gaviero/settings.json` can loosen what the workspace or user file sets.

`/settings` opens the global file in an editor tab labelled `global:settings.json`, drawn in violet because every gaviero instance under your user account reads it (it is created as `{}` when missing). `/settings workspace` and `/settings repo` open the other two layers (`repo` = the active buffer's folder). In any open layer, a key that another layer also sets is highlighted: struck out and grey when a more specific layer overrides it, amber and underlined when it overrides a broader one, with a note at the end of the line naming the layers (`← overridden by repo gaviero`, `overrides global`; a `key+` reads `adds to global` / `← extended by repo gaviero` and is not struck out). Keys are compared by dotted path, nested and flattened (`"a.b": …`) alike; keys inside arrays are not compared. Saving a layer reloads the settings and refreshes the highlights in the other open layers.

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
| `agent.permissions.read.extraFolders` | Folders outside the workspace that agents may read but not edit — absolute, `~/…`, or relative to the folder holding `.gaviero/settings.json`. They are not workspace folders: no turn review, indexing, memory scope or `GAVIERO.md` discovery. Entries that don't exist, or that lie inside or contain a workspace folder, are skipped. | none |
| `agent.permissions.scratchFolders` | Folders outside the workspace where agents work as freely as in a workspace folder, without the outside-workspace prompt — for downloaded packages, cloned repos, git worktrees and experiments. Paths resolve like `read.extraFolders`. Changes there are **not** part of turn review, and the folders get no indexing, memory scope or `GAVIERO.md` discovery; the sensitive-file and hard-floor rules still apply. Entries that don't exist, that lie inside or contain a workspace folder, or that lie inside a `read.extraFolders` entry are skipped. | none |
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
        "blockOutsideWorkspace": true,
        "extraFolders": ["../shared-docs", "~/reference/api-specs"]
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
| Outside the workspace | Anything outside the workspace folders, while `agent.permissions.read.blockOutsideWorkspace` is `true` (the default) | Set it to `false`, or list specific folders in `agent.permissions.read.extraFolders` (read-only; the built-in sensitive files and the hard floor stay blocked inside them) |

`.gaviero/settings.json`, `.gaviero/skills/` and `.gaviero/tmp/` stay readable; spilled prompts live in `.gaviero/tmp/`.

These rules come from the same three settings layers as everything else (user, workspace, repo). In a multi-folder workspace the workspace's `.gaviero` folder, beside the `*.gaviero-workspace` file, usually lies outside every workspace folder, so agents get it as a read-only folder (like `agent.permissions.read.extraFolders`) and can check the settings without a permission prompt. Its hard-floor entries stay blocked.

**How each provider gets the policy:**

| Provider | How the policy is applied |
|---|---|
| Claude | `--setting-sources user,project` plus a `--settings` layer of `Read(...)` deny rules (they cover the Read tool, Grep and Bash `cat`) and the outside-workspace block. A `PostToolUse` Bash hook (`gaviero-cli --hook claude:post-tool-use`) withholds output lines from protected files, e.g. `grep -r` hits, and redacts secret-looking values. A startup chat message lists any `~/.claude/settings.json` allow rules that would skip gaviero's prompts. |
| Cursor | `.cursor/hooks.json`, written in each workspace folder at startup (`beforeReadFile`, `beforeShellExecution` → `gaviero-cli --hook cursor:…`). Cursor's `cli.json` `Read()` denies do not stop reads. |
| Codex (chat) | A deny-read permission profile in `thread/start`, running under the MXC sandbox on Windows. If Codex cannot enforce it, the session fails closed with an explanation. Shell environment variables named `*KEY*`, `*SECRET*` or `*TOKEN*` are filtered out. |
| `deepseek:` | Enforced in-process, Bash file arguments included. |

`agent.permissions.read.extraFolders` reaches each provider as: Claude — `--add-dir` per folder plus `Edit(//folder/**)` denies (they cover Edit, Write and MultiEdit, not Bash) and the sensitive-file denies anchored there; Codex chat — a `"read"` entry per folder in the permission profile (sensitive-file globs are not repeated under it); `deepseek:` — Read, Grep and Glob accept paths there, writes stay workspace-only. Every provider is told about the folders and that they are read-only in its system prompt; Cursor and `dsh:` get only that notice. A Bash command that writes into one of these folders is not caught by turn review, which only covers workspace folders.

`agent.permissions.scratchFolders` reaches each provider as: Claude — `--add-dir` per folder with no `Edit` deny, plus the sensitive-file denies anchored there; Codex chat — a `"read"` sandbox entry per folder and file changes accepted there (shell writes ask, as in workspace folders); `deepseek:` — Read, Grep, Glob, Write, Edit and MultiEdit accept paths there. Every provider is told in its system prompt to put downloads, clones, worktrees and experiments there. Cursor and `dsh:` get only that notice (`dsh:` cannot mount extra directories). Example: `"agent": {"permissions": {"scratchFolders": ["~/gaviero-scratch"]}}`.

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
| `claude-requests.ndjson` | One row per Claude API request: session, model, size, SHA-256, and any secret-looking values found (redacted excerpts only), each with its `classification` and, when known, the `source` file it was read from. The raw bodies are deleted after the scan. |
| `hooks.ndjson` | Every Claude and Cursor hook decision: the path or command, and whether it was allowed, denied or redacted, with the reason. |

Only Claude's request contents are captured; for the other providers the log shows which hosts they reached, not what they sent.

**Fixture classification.** Each secret-looking value in a Claude request is traced to the file it came from: the `Read` call behind a tool result, or the `path:` prefix of a `grep` line. It is then classified, checking in this order:

| Classification | When |
|---|---|
| `allowlisted` | The value's line carries an allowlist marker: `pragma: allowlist secret`, `gitleaks:allow` or `trufflehog:ignore`. |
| `test-path` | The source is test code or data: a `tests/`, `test/`, `__tests__/`, `fixtures/`, `testdata/` or `spec/` folder; a name like `*_test.*`, `test_*.py`, `*.test.*`, `*.spec.*` or `conftest.py`; or, in a Rust file, every occurrence sits after `#[cfg(test)]`. |
| `dummy-pattern` | The value doesn't look random: sequential (`ABCDEFGH…`, `123456…`), repetitive (`aaaa…`, few distinct characters), a placeholder word (`EXAMPLE`, `dummy`, `fake`, `test`, `canary`, …), or a PEM block with a tiny body. |
| `unclassified` | None of the above. |

Only `unclassified` values get a chat warning each. The others are summed into one line ("N fixture-like secret values … not shown") and kept in full in the log. These signals are hints, not proof: an agent or a planted file can add a marker, so nothing is dropped.

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
