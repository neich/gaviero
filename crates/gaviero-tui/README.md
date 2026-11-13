# gaviero-tui

## Overview

Interactive terminal editor and workspace for Gaviero. A multi-tab code editor, file tree, git integration, agent chat, turn history, and embedded terminal in one full-screen TUI. All execution logic lives in [`gaviero-core`](../gaviero-core/README.md); this crate handles rendering and input only.

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
gaviero --folder /path/to/dir                  # plain folder, even if it holds a workspace file
gaviero --sessions                             # session host: reopen the last set of sessions
gaviero --sessions /path/to/repo               # session host with only this session
gaviero --pick [PATH]                          # choose the folder in a folder browser first (starts at PATH)
```

`gaviero --sessions` runs several gaviero sessions in one terminal, each a full `gaviero` process, under a one-row tab bar. Alt+Left / Alt+Right switch sessions (or click a tab), Alt+Shift+Left / Alt+Shift+Right move the current tab one place (the order is kept for the next start), Alt+N opens a new one in a folder browser over the whole disk (the empty box lists recent paths, the start folder, home and every drive; type a folder / `.gaviero-workspace` path with Tab completion, → enters the selected folder, ← goes to the parent, Enter opens the selection — a listed folder shows itself first), Alt+Q or a click on the `x` at the start of a tab closes that tab and ends its gaviero (it saves its session state; a session that has not exited after 3 s is killed), and the tab before it becomes current. Ctrl+Q quits the host the same way, keeping every tab for the next start; inside the host Ctrl+Q never reaches a session. (Alt+X is not used: Windows keeps it.) Both act at once unless something would be lost — an agent, a swarm or a shell command running, or unsaved files — in which case a popup lists it per tab and only `y` goes ahead. Every tab, the current one included, ends in two badges, a letter on a rounded pill of the state's colour. `A` is the agent's: green while an agent or a swarm runs, yellow while it waits for you (a permission prompt, an agent's question or a turn review), blue once it finished while you were elsewhere. `S` is the shells': green while a command runs in one of the session's shells, yellow when that command seems to wait for you, blue once one finished while you were elsewhere. Yellow is certain when the program rings the bell or sends a desktop notification (OSC 9, 777 or 99); otherwise it is a guess, made after 5 seconds with no output and no typing: a full-screen program sitting idle (vim, less), or a line left unfinished like `Password: ` or `[y/N] `. A long silent step that leaves a half-written line can show yellow by mistake. Typing in that shell turns it green again (seen through the shell integration gaviero injects into bash, zsh, fish and PowerShell 7.2+; other shells never show it). Making a tab current keeps both; a finished badge clears when you leave the tab again, or as soon as new work starts in it. A tab closes when its gaviero exits; the host exits after the last one. Without a path the host reopens the sessions open when it last ended, else the current directory. Only one host runs at a time: a second `gaviero --sessions` (for example from a shell profile in a new terminal) prints a warning and exits without opening anything — switch to the running host instead. When a new build is installed (`scripts/deploy.ps1`), the host restarts itself and every tab onto it once no tab has an agent, a swarm or a shell command running or unsaved files and you have not typed into it for 15 s; the same tabs reopen, with the same one current. Until then the tab bar says what it is waiting for. A host started with `GAVIERO_NO_SUPERVISOR` (or without a terminal) cannot restart itself and says so once. Windows is the tested platform.

**SSH tabs.** In the Alt+N picker, Ctrl+T switches between **Local** and **SSH**. In SSH mode type `host` or `host:/path` (`host:~/path` works too); the list offers the `Host` aliases from `~/.ssh/config` and recent SSH tabs. The tab runs Windows' own OpenSSH (`%SystemRoot%\System32\OpenSSH\ssh.exe`; `ssh` elsewhere), which starts gaviero on that machine through your login shell, so keys, mouse, tab badges and the close guard work as in a local tab, and ssh's own prompts (password, host key) appear in the tab. With no path the remote gaviero opens its folder browser (`gaviero --pick`) and the tab takes the folder you choose there as its name (`host:folder`) and for reopening. gaviero must be installed on the remote; when it is not on the login PATH, set its location in the host state file (`session-host.json` in gaviero's data folder — `%APPDATA%\gaviero\` on Windows, `~/.local/share/gaviero/` on Linux): `"sshHosts": { "myhost": { "gaviero": "~/.cargo/bin/gaviero" } }`. Closing an SSH tab ends the connection at once and the remote gaviero saves its session as it exits. A dropped connection keeps the tab with a **Disconnected** box: Enter reconnects (the remote session is restored; an agent turn that was running is lost), Alt+Q closes it. Text the remote gaviero copies lands on this machine's clipboard, and Ctrl+V in an SSH tab pastes this machine's clipboard text (images do not cross). Remote machines must have a POSIX login shell (sh, bash, zsh, fish); paths containing `'`, `"`, `\`, `$` or a backtick are refused.

Opening a directory that holds a `.gaviero-workspace` file asks first whether to open that workspace (preselected) or the directory as a plain folder (Esc). A plain folder keeps its own memory, code graph and turn history under its `.gaviero/` and ignores the members' `.gaviero/settings.json`. `--folder` skips the question, `--workspace` opens the workspace directly, and a restart onto a new build reuses the answer.

In a folder without `.gaviero/settings.json` — or a `.gaviero-workspace` file without one beside it — a setup screen offers an editor-only session (nothing written), a minimal `{}` settings file, or full settings — a copy of the global `$GAVIERO_ROOT/.gaviero/settings.json` (built-in full settings when it does not exist).

**Panel focus:** Alt+1/2/3/4 (left/editor/side/terminal). **Side panels:** Alt+A (chat), Alt+W (swarm), Alt+H (history), Alt+G (git — one tab per repository: the one containing each opened folder plus every repository cloned directly inside it, so a plain folder of clones works without a workspace file; Alt+O/I switch tabs while the panel is focused), Alt+M (memory), Alt+R (code graph for the active file). **Code graph:** F12 / Alt+. / Ctrl+click go to the definition under the cursor (a visible CODE GRAPH panel follows the jump); Alt+S shows the CODE GRAPH panel for the symbol under the cursor without leaving the editor. The panel scrolls with the wheel once focused (click it or Alt+3) and has a scrollbar you can click or drag. **Left panel:** Alt+E (explorer), Alt+F (find — Tab switches between file contents and file names; gitignored folders are skipped), Alt+C (changes).

## Examples

**Review a file** (agent chat, Alt+A):

```
review src/auth/session.rs for race conditions
```

**Switch model:**

```
/model claude:opus
```

**Inspect what a turn did** (history panel, Alt+H): pick a turn, then `Tab` / `1`–`6` through PROMPT, TOOLS, MCP, MEMORY, TOTALS, FILES (what the turn changed on disk and how its review resolved; `u` twice restores a reviewed turn's files, last 5 turns only); `Enter` expands a section, `/` filters, `a` toggles all / active conversation, `r` reloads, `c` copies the section's records. Token numbers prefixed `~` are estimates (words×1.3 for text, chars÷4 for JSON); `exact` numbers are provider-reported. The same log is readable headless with `gaviero-cli --history`.

**Run a DSL workflow** (SWARM panel, Alt+W): pick a script from `.gaviero/dsl/<name>/<name>.gaviero` or type a path, fill in its inputs, and move down to **Run**. A list input (a roster or a `for` list) shows one editable row per item: click the `x` before an item or press `Backspace` on an empty one to remove it, and click the `+` after `(list)` or press `Alt+Enter` to add one. While it runs, `↑`/`↓` choose an agent and its output shows below; `Enter` expands the output, `Tab` scrolls it, and `x` twice stops the run. Afterwards, `d` shows an agent's diff, `u` twice undoes the run's merges, and `n` starts another. The same scripts run headless with `gaviero-cli --script`.

**Turn review** — agents edit files directly while they work (edit tools, shell commands, formatters — whatever they need to test). When a turn ends, gaviero compares the workspace with a snapshot taken before the prompt (no git needed; `.gitignore` and `files.exclude` paths are skipped, except the gitignored folders you list in `turnReview.includeIgnored`, e.g. `"turnReview": { "includeIgnored": ["plans", "docs/private"] }` — paths relative to each workspace folder; gaviero's own `.gaviero/` folder is skipped too, except `settings.json`, `dsl/` and `skills/`, and any other folder in it you list there, such as `".gaviero/plans"`) and, if anything changed, opens the TURN REVIEW panel. That conversation cannot send its next prompt until every changed file is accepted or rejected; other conversations keep working. Accepted edits stay as the agent left them; rejected ones go back to the pre-prompt version. Sensitive files (`.env`, keys) an agent changed are restored automatically. Cancelled and failed turns are reviewed too.

Four decisions, each applied immediately — the review ends when every file has one:

| Key | Action |
|---|---|
| `a` / `r` | Accept / reject the selected file (reject = back to its pre-prompt version) |
| `A` / `R` | Accept / reject the whole turn (every file not decided yet) |

`j` / `k` move between files, `J` / `K` scroll the diff, `Tab` switches between pending reviews. Rejecting a file you changed after the turn asks you to press the key again.

Pending reviews survive a restart. The phone (remote) has the same four decisions. When an agent proposes changes outside chat turns (or turn capture is unavailable), the classic Write Gate diff overlay opens instead: `]h`/`[h` navigate, `a`/`r` accept/reject a hunk, `A`/`R` all, `f` finalize, `q` exit.

### Chat commands

| Command | Purpose |
|---|---|
| `/model <spec>` | Switch model (`claude:sonnet`, `deepseek:deepseek-flash`, `dsh:deepseek-flash`, …) |
| `/remember <text>` | Store a fact (`-here`, `-module`, `-workspace`, `-global` scope it) |
| `/forget <query>` | Soft-delete matching memories |
| `/skills [search <q>]` | List or search loaded skills |
| `/mcp` / `/mcp probe` | MCP reach record + endpoint; probe pointer (CLI) |
| `/attach <path>` / `/detach` | Add/remove file context |
| `/add-dir [path]` | Let the agent read a folder outside the workspace (read-only) for this conversation until `/reset`, `/clear` or `/handoff`; bare form lists the grants. Enforced for `claude:`, `codex:`, `cursor:` (prompt only, like `read.extraFolders`) and `deepseek:`; not for `dsh:`, `ollama:`, `local:`. Desktop only. |
| `/lite` | Minimal-context turn (topology only) |
| `/compact [text\|N]` | Claude: the CLI compacts its own session (optional extra guidance). Other providers: keep the last N messages of the transcript gaviero replays |
| `/clear` | Clear agent context (alias of `/reset`) |
| `/keep [clear]` / `/handoff [to <tab\|new>]` | Mark exchanges to keep in browse mode (Ctrl+C with nothing selected, ↑/↓, **Space** → `★`), then `/handoff` starts a fresh session carrying only those exchanges, verbatim; detours are left out. `/handoff to <n\|title>` seeds an existing tab instead (the source is left untouched), `/handoff to new` opens one. `/keep` lists them. |

Chat input supports `$skill` invocation with `$`-prefix autocomplete. Full slash inventory: [CLAUDE.md](CLAUDE.md).

### Keybindings (selected)

| Keys | Action |
|---|---|
| Ctrl+B / Ctrl+P / Ctrl+J | Toggle file tree / side panel / terminal |
| Ctrl+S / Ctrl+Z / Ctrl+Y | Save / undo / redo |
| Ctrl+F / F3 | Find in file / workspace search |
| Ctrl+F (markdown preview-only, Alt+P) | Find in the rendered preview: the bar takes the preview's top row; Enter / Down / F3 next, Up previous, Esc closes. A match hidden in a collapsed section expands the sections |
| F5 / F7 | Format buffer / toggle word wrap |
| F10 / Shift+F10 | Fold block at cursor / fold all — or click the `▾` in the gutter |
| F10 / Shift+F10 (markdown preview-only, Alt+P) | Fold the `#`–`###` section at the top of the preview / step the outline: `#` only → down to `##` → down to `###` → all expanded. Clicking a heading folds it in split view too |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Alt+Up/Down (Ctrl+Up/Down fallback) | Resize terminal split / chat input height |
| Ctrl+Alt+Left/Right | Resize explorer / editor / side panel widths |
| Double-click (explorer) | On a file the editor cannot open as text (images, PDFs, archives, …): show it selected in the OS file manager (Explorer / Finder / the freedesktop file manager) so you can open it with another app |
| Alt+Left/Right (`--sessions` host) | Previous / next session |
| Alt+Shift+Left/Right (`--sessions` host) | Move the current tab one place left / right (stops at the ends; the order is kept for the next start) |
| Alt+N (`--sessions` host) | New session picker; Ctrl+T in the picker switches Local / SSH (`host` or `host:/path`) |
| Ctrl+V (SSH tab, `--sessions` host) | Paste this machine's clipboard into the remote session |
| Alt+Q (`--sessions` host) | Close the active tab and end its gaviero (or click the `x` on a tab; asks first when work is in progress) |
| Ctrl+Q (`--sessions` host) | Quit the host; every tab reopens next start (asks first when work is in progress) |

F7 is the reliable word-wrap chord on every host. See [Root README](../../README.md) for the full editor keybinding tables.

## Configuration

Settings layers, each overriding the one before it key by key:

1. User — `$GAVIERO_ROOT/.gaviero/settings.json` (`~/.gaviero/settings.json` when unset; the old XDG/AppData path is not read)
2. Workspace — `.gaviero/settings.json` beside the `*.gaviero-workspace` file (the folder's own file in single-folder mode)
3. Repo — each workspace folder's `.gaviero/settings.json`, for that folder

Built-in defaults fill the keys no layer sets. A key written with a trailing `+` (`"mcp.extraServers+": [...]`) adds to the inherited value instead of replacing it — lists append (same `"name"` replaces), objects merge — see the root README. Settings embedded in the `.gaviero-workspace` file are not read.

`/settings` opens the global file in an editor tab labelled `global:settings.json`, drawn in violet because every gaviero instance under your user account reads it (it is created as `{}` when missing). `/settings workspace` and `/settings repo` open the other two layers (`repo` = the active buffer's folder). In any open layer, a key that another layer also sets is highlighted: struck out and grey when a more specific layer overrides it, amber and underlined when it overrides a broader one, with a note at the end of the line naming the layers (`← overridden by repo gaviero`, `overrides global`; a `key+` reads `adds to global` / `← extended by repo gaviero`). Keys are compared by dotted path, nested and flattened (`"a.b": …`) alike; keys inside arrays are not compared. Saving a layer reloads the settings and refreshes the highlights in the other open layers.

A settings file that is not valid JSON contributes nothing to the cascade. At startup a modal names the file, line, column, and parser message: `q` quits, `Enter` continues and opens the file with the cursor at the error.

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

Remote access goes through [gaviero-relay](../gaviero-relay/README.md) only: run it behind nginx on one subdomain of a VPS you control, then put the same two keys on every machine (`~/.gaviero/settings.json`). Without `remote.relay.host` the sidecar does not start.

```json
{
  "remote": {
    "relay": {
      "host": "gaviero.example.com",
      "secret": "<the relay's --secret-file contents>"
    }
  }
}
```

- **gaviero dials out:** the listener binds loopback only, and gaviero registers `<workspace-id>-<machine>.<host>` with the relay.
- **The relay stays blind:** the phone reaches the instance by SNI through the relay, with TLS end to end against a per-machine CA (`~/.gaviero/remote/tls/pinned/`) that the app pins from the pairing QR.
- **Notifications go to the relay:** AgentFinished/AgentWaiting are sent there for the app's background alert service.
- **Optional keys:** `remote.enabled` (default `true`), `remote.port` (default: derived from the workspace), `remote.tokenScope` (`"machine"` or `"workspace"`), `remote.relay.machine` (default: the hostname), `remote.relay.url` (default `wss://<host>`). The bearer token lives in `~/.gaviero/remote/` unless `tokenScope` is `"workspace"`.

Full setup: [docs/remote-relay.md](../../docs/remote-relay.md).

### Agent notifications

Two chat milestones announce themselves, each with its own `enabled` / `desktop` / `statusBar` switches (all default `true`). Notifications are silent: gaviero never rings the terminal bell or plays a system sound.

| Setting group | Fires when |
|---|---|
| `notifications.agentFinished.*` | The turn ended — an answer (or an error) is on screen |
| `notifications.agentWaiting.*` | The turn is blocked on you — a tool permission prompt or an `AskUserQuestion` |

The two use different glyphs and banner colours (`✓` green vs `?` amber) so they are distinguishable at a glance.

- **Desktop toasts fire only while the terminal is unfocused** — with gaviero on screen the status-bar banner already carries the news. (No-op on Windows: see `spawn_desktop_notification`.)
- **The status-bar banner and fullscreen toast** are drawn while you are looking, for 8s.

```json
{
  "notifications": {
    "agentFinished": { "desktop": false },
    "agentWaiting": { "statusBar": true }
  }
}
```

Phone alerts for the same two milestones go through gaviero-relay (see above) and reach the Gaviero Remote app's background alert service even when the phone is locked.

## API

The TUI is a binary, not a library. Internally it implements the `WriteGateObserver`, `AcpObserver`, memory and MCP observer traits from `gaviero-core`, bridging core callbacks into an `mpsc` event channel.

**Event-loop rule:** `draw → recv event → handle → repeat`. No background task mutates `App` directly.

| Module | Role |
|---|---|
| `app.rs` + `app/` | `App` struct, layout, focus, slash commands |
| `editor/` | Ropey buffer, tree-sitter highlight, diff overlay, word wrap |
| `panels/` | `agent_chat`, `history_panel`, `git_panel`, `memory_panel`, `terminal`, … |
| `keymap.rs` / `event.rs` | Keybindings and event variants |
| `platform.rs` | Windows/ConPTY, AltGr, paste coalescing |

## See Also

- [CLAUDE.md](CLAUDE.md) — event loop, panel patterns, full slash list
- [Root README](../../README.md) — feature overview
- [gaviero-core](../gaviero-core/README.md) — runtime the TUI drives

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
