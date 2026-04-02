# gaviero-tui

A terminal code editor with integrated AI agents and multi-agent swarm execution. Opens a full-screen TUI with a file tree, multi-tab editor, agent chat, swarm dashboard, git panel, and embedded terminal — all in one window.

---

## Layout

```
┌─────────────┬──────────────────────────────────────┬───────────────────┐
│ Left panel  │  Editor (tabbed)                     │  Side panel       │
│             │                                      │                   │
│  File tree  │  code.rs   main.rs   README.md  +   │  Agent chat       │
│  Search     │                                      │  Swarm dashboard  │
│  Review     │  ... file content ...                │  Git panel        │
│  Changes    │                                      │                   │
├─────────────┴──────────────────────────────────────┴───────────────────┤
│ Terminal (toggleable, resizable)                                        │
└─────────────────────────────────────────────────────────────────────────┘
```

Each panel has multiple modes. Switch modes with `Alt+<key>` shortcuts or cycle with the arrows shown in panel headers. The three columns resize via six built-in layout presets (`Alt+5` – `Alt+0`).

---

## Keyboard shortcuts

### Global

| Key | Action |
|---|---|
| `Ctrl+Q` | Quit |
| `Ctrl+S` | Save current file |
| `Ctrl+B` | Toggle file tree |
| `Ctrl+P` | Toggle side panel |
| `Ctrl+J` / `F4` | Toggle terminal |
| `Ctrl+T` | New editor tab |
| `Ctrl+W` | Close editor tab |
| `Alt+[` / `Alt+]` | Previous / next tab |
| `Ctrl+F` | Find in buffer |
| `F5` | Format buffer (at current level) |
| `F6` | Cycle format level (Compact → Normal → Expanded) |
| `Alt+P` | Toggle markdown preview |

### Panel focus

| Key | Panel |
|---|---|
| `Alt+1` | Left panel |
| `Alt+2` | Editor |
| `Alt+3` | Side panel |
| `Alt+4` | Terminal |

### Left panel mode

| Key | Mode |
|---|---|
| `Alt+E` | File tree |
| `Alt+F` / `F3` | Search |
| `Alt+C` | Working-directory changes |

### Side panel mode

| Key | Mode |
|---|---|
| `Alt+A` | Agent chat |
| `Alt+W` | Swarm dashboard |
| `Alt+G` | Git panel |

### Layout presets

| Key | Widths (tree / editor / side) |
|---|---|
| `Alt+5` | 15 / 70 / 15 — standard |
| `Alt+6` | 15 / 85 / 0 — editor focus |
| `Alt+7` | 0 / 100 / 0 — full editor |
| `Alt+8` | 0 / 60 / 40 — code + notes |
| `Alt+9` | 25 / 75 / 0 — wide tree |
| `Alt+0` | 20 / 55 / 25 — three columns |

---

## Editor

Multi-tab editor with syntax highlighting (tree-sitter), undo/redo, word-aware movement, and clipboard support.

### Editing keys

| Key | Action |
|---|---|
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+A` | Select all |
| `Ctrl+K` | Delete line |
| `Ctrl+D` | Duplicate line |
| `Alt+↑` / `Alt+↓` | Move line up / down |
| `Ctrl+←` / `Ctrl+→` | Move by word |
| `Ctrl+Shift+←` / `Ctrl+Shift+→` | Select word |
| `Shift+↑↓←→` | Extend selection |
| `Ctrl+E` | Jump to line end |
| `Ctrl+H` | Delete word backward |
| `Ctrl+Backspace` | Delete word back |
| `Ctrl+Delete` | Delete to end of line |
| `Home` / `End` | Line start / end |
| `PageUp` / `PageDown` | Scroll by page |

### Format levels

Press `F6` to cycle the format level, then `F5` to apply:

| Level | Effect |
|---|---|
| **Compact** | Fix indentation only; preserve existing single-line constructs |
| **Normal** | Fix indentation and normalize spacing; break long lines |
| **Expanded** | Full reformat via external tool (rustfmt, prettier, etc.) |

### Markdown preview

`Alt+P` opens a rendered preview pane next to the editor for `.md` files, including thinking blocks from AI responses.

---

## Left panel

### File tree (`Alt+E`)

Browse and manage the workspace directory structure.

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate |
| `Enter` / `←` / `→` | Open file or expand/collapse directory |
| `Space` | Toggle directory expansion |
| `n` | New file (prompts for name) |
| `N` | New folder |
| `r` | Rename |
| `d` / `Delete` | Delete (confirm with `y`) |

Single-child directory chains are compacted into one entry. `.gitignore`-style exclusions apply.

### Search (`Alt+F` or `F3`)

Live full-text search across all workspace files.

- Type to search — results update as you type (debounced)
- Results show `file:line — matching content`
- `Down` / `Enter` — move focus to results
- `↑` / `Esc` — return to input
- `Enter` on a result — open the file at that line

### Changes (`Alt+C`)

Working-directory git changes browser. Shows unstaged and staged files with status markers (`M` modified, `A` added, `D` deleted, `R` renamed). Open any file's diff from here.

### Review mode

When an agent produces multi-file proposals, the left panel switches to **Review** mode automatically — a list of all proposed files with `+lines/-lines` summaries. Navigate with `↑↓`, accept or reject individual files, then apply accepted changes.

---

## Side panel

### Agent chat (`Alt+A`)

Streaming AI conversation panel. Type at the bottom, `Alt+Enter` or `Shift+Enter` to send.

**Slash commands:**

| Command | Description |
|---|---|
| `/swarm <task>` | Multi-agent swarm: plan and execute in parallel |
| `/cswarm <task>` | Coordinated swarm: Opus generates a `.gaviero` plan file for review, then you run it with `/run` |
| `/run <file.gaviero>` | Compile and execute a DSL script |
| `/undo-swarm` | Revert the last swarm run |
| `/remember <text>` | Store text to persistent semantic memory |
| `/attach [path]` | Attach a file to the conversation (supports `@` autocomplete) |
| `/detach <name>\|all` | Remove an attachment |
| `/help` | Show available commands |

**File references:** Type `@` in the input to get an autocomplete popup for workspace files. The selected path is inlined into your message as context for the agent.

**Model selector:** The chat header shows the active model (Opus / Sonnet / Haiku) and effort level. Context percentage indicates how much of the model's context window is in use.

### Swarm dashboard (`Alt+W`)

Real-time monitor for multi-agent swarm runs.

- Shows each agent's status (Pending / Running / Completed / Failed)
- Displays phase progression (coordinating → executing → merging)
- For `/cswarm`: shows the path of the generated `.gaviero` plan file
- Press `Enter` on a completed agent to review its diff
- `/undo-swarm` to revert the entire run to the pre-swarm git state

### Git panel (`Alt+G`)

In-TUI git operations: stage, unstage, discard, commit, and amend without leaving the editor.

- Three regions: Unstaged / Staged / Commit message
- `Tab` to cycle between regions
- Stage and unstage files, view diffs, write commit message
- Branch display with `/` to open branch picker (filterable)

---

## Diff review

When an agent proposes file changes, a diff overlay appears in the editor. All writes are gated — nothing is written to disk until you accept.

| Key | Action |
|---|---|
| `]h` | Next hunk |
| `[h` | Previous hunk |
| `a` | Accept current hunk |
| `r` | Reject current hunk |
| `A` | Accept all hunks |
| `R` | Reject all hunks |
| `f` | Finalize — write all accepted hunks to disk |
| `q` | Dismiss — discard all proposed changes |
| `↑↓` / `j k` | Scroll line by line |
| `PageUp` / `PageDown` | Scroll by page |

The overlay shows added lines in green, removed lines in red, and context in gray. The gutter shows which function or struct each hunk belongs to. The status bar displays `[accepted/total]` and navigation hints.

**Batch review:** When an agent changes multiple files, the left panel enters Review mode (file list with summaries). The same `a`/`r`/`f` keys apply per-file. `Ctrl+←/→` switches focus between the file list and the diff.

---

## Terminal (`Ctrl+J` or `F4`)

Full PTY terminal embedded at the bottom of the window.

| Key | Action |
|---|---|
| `Ctrl+J` / `F4` | Show / hide terminal |
| `Alt+↑` / `Alt+↓` | Resize terminal split |
| `Shift+PageUp` / `Shift+PageDown` | Scroll terminal scrollback |
| `Alt+1` – `Alt+4` | Switch focus back to TUI panels |

Supports vt100 colour, mouse selection (click-drag), and multiple terminal tabs. All keys pass through to the shell while the terminal is focused.

---

## Session persistence

Gaviero restores the following state on restart:

- Open tabs, active tab, and cursor / scroll position per tab
- Panel visibility and active modes
- File tree expanded directories
- Terminal split size
- Active layout preset
- Chat conversations (per workspace)
