# gaviero-tui

A terminal code editor with integrated AI agents and multi-agent swarm execution. Opens a full-screen TUI with a file tree, multi-tab editor, agent chat, swarm dashboard, git panel, and embedded terminal — all in one window.

---

## Layout

```
┌─────────────┬────────────────────────────┬──────────────────────┐
│  Left panel │        Editor (tabs)        │     Side panel       │
│             │                            │                      │
│  File tree  │  syntax highlighting       │  Agent chat          │
│  Search     │  gutter + scrollbar        │  Swarm dashboard     │
│  Review     │  hunk overlay (diff)       │  Git panel           │
│  Changes    │  markdown preview          │                      │
├─────────────┴────────────────────────────┴──────────────────────┤
│  Embedded terminal (PTY)                                        │
├─────────────────────────────────────────────────────────────────┤
│  Status bar                                                     │
└─────────────────────────────────────────────────────────────────┘
```

Panels can be toggled, resized, and hidden. Six layout presets are available via `Alt+5`–`Alt+0`.

---

## Starting

```bash
gaviero                    # open in current directory
gaviero /path/to/repo      # open at specific path
```

---

## Workflow walkthroughs

### 1. Fix a bug (single agent, iterative refinement)

1. Open the chat panel — `Alt+A`
2. (Optional) Attach the relevant file — type `@src/auth.rs` or use `/attach src/auth.rs`
3. Type your request and press `Enter`:
   ```
   Fix the null pointer crash in UserService.getById when the user does not exist
   ```
4. The agent streams its reasoning and edits. Tool calls appear in the activity log.
5. When the agent proposes file changes, the left panel switches to **Review** mode automatically.
6. Navigate hunks with `]h` / `[h`, accept with `a`, reject with `r`.
7. Press `f` to finalize (write accepted changes to disk).
8. Switch to the terminal (`Ctrl+J`), run `cargo test` to verify.

---

### 2. TDD bug fix — generate failing tests first

1. Open chat — `Alt+A`
2. Send:
   ```
   Write a test that reproduces this bug: tokens are not invalidated on logout.
   The test must fail against the current code. Do not modify any source files yet.
   ```
3. Review the proposed test file — accept it with `A` then `f`.
4. Confirm the test fails: switch to terminal (`Ctrl+J`), run `cargo test`.
5. Send a follow-up:
   ```
   Now fix the code so the test passes. Do not modify the test file.
   ```
6. Review proposed changes hunk-by-hunk, accept, finalize.
7. Run `cargo test` again to confirm green.

Alternatively use `/run` with a dedicated script (see §Run a workflow script below).

---

### 3. New feature with coordinated planning (`/cswarm`)

Use this when you want Opus to decompose a large task into a multi-agent plan before anything runs.

1. Open chat — `Alt+A`
2. Type:
   ```
   /cswarm Add subscription billing with proration support
   ```
3. Opus decomposes the task. The generated `.gaviero` file opens in the editor automatically.
4. Read the plan. Edit agent prompts, scopes, `depends_on` edges, or `context {}` blocks directly in the editor.
5. Save — `Ctrl+S`.
6. Execute:
   ```
   /run tmp/gaviero_plan_<timestamp>.gaviero
   ```
7. Switch to the swarm dashboard — `Alt+W` — to watch agents run.
8. When complete, each agent's proposed changes appear in the review panel. Accept or reject per-hunk.

---

### 4. Immediate multi-agent swarm (`/swarm`)

Skip the planning review step and run agents immediately from a natural-language task description.

1. Open chat — `Alt+A`
2. Type:
   ```
   /swarm Rename ModelTier variants: Execution→Cheap, Coordinator→Expensive, across all crates
   ```
3. The swarm dashboard opens (`Alt+W`) showing each agent's status.
4. Watch the activity log: tool calls, file writes, validation results.
5. When all agents complete, merge results are applied automatically (with conflict resolution if needed).
6. Proposed changes open in the review panel — accept with `A`, finalize with `f`.

---

### 5. Run a workflow script (`/run`)

For repeatable workflows defined in `.gaviero` files:

1. Open the file tree — `Alt+E`, `Alt+1`
2. Navigate to `workflows/bugfix.gaviero`, press `Enter` to open it in the editor
3. Switch to chat — `Alt+A`
4. Type:
   ```
   /run workflows/bugfix.gaviero
   ```
   Or with a runtime prompt substituted for `{{PROMPT}}`:
   ```
   /run workflows/bugfix.gaviero fix null pointer in UserService.getById
   ```
5. The iteration engine runs the workflow. Validation feedback (compile, clippy, test) appears in the swarm dashboard.
6. Review proposed changes when complete.

---

### 6. Security audit (read-only, no changes)

1. Open chat — `Alt+A`
2. Type:
   ```
   /run workflows/security_audit.gaviero
   ```
   Or directly:
   ```
   Audit src/auth/ for injection vulnerabilities and write findings to docs/security-audit.md
   ```
3. When the agent finishes, `docs/security-audit.md` opens in a new tab.
4. Accept the report with `A` then `f`.

To store audit findings in memory for future runs:
```
/remember The auth module uses RS256 JWT with 1-hour expiry, refresh tokens stored in Redis
```

---

### 7. Memory-assisted development

Agents automatically query memory before each task. You can also store context manually:

```
/remember The billing module uses Stripe webhooks for all payment events — never poll the API directly
```

```
/remember UserService.getById returns Option<User>, not Result — callers must handle None
```

On the next agent turn, these facts appear in the prompt context. Memory persists across sessions and is scoped per workspace.

To override which namespace agents write to:
```
/namespace auth-team
```

---

### 8. Reviewing and editing a coordinated plan

After `/cswarm` generates a plan, the `.gaviero` file opens in the editor. Common edits before running:

**Change a prompt:** Find the `prompt` field and edit it directly.

**Narrow scope:** Change `owned ["."]` to `owned ["src/billing/"]` to prevent an agent from touching unrelated files.

**Enable blast-radius expansion:**
```gaviero
scope {
    owned        ["src/billing/"]
    impact_scope true     // auto-include files that call into owned paths
}
```

**Add code-graph context queries:**
```gaviero
context {
    callers_of ["src/billing/invoice.rs"]
    tests_for  ["src/billing/"]
    depth      2
}
```

**Add verification:**
```gaviero
workflow my_plan {
    steps [agent-a agent-b]
    verify { compile true impact_tests true }
}
```

**Change strategy to best-of-3 for a risky agent:**
```gaviero
workflow my_plan {
    steps    [risky-agent]
    strategy best_of_3
    max_retries 4
}
```

Save and `/run` the edited file.

---

### 9. Diff review workflow in detail

When an agent proposes changes, the left panel switches to **Review** mode. The editor shows the diff inline (added lines green, removed lines red).

```
Left panel (Review):          Editor (diff view):
─────────────────────         ──────────────────────────────
src/auth.rs  +12 -3           fn get_user(id: u64) -> Option<User> {
tests/auth_test.rs  +45 -0  -     let user = db.find(id);
                            -     user
                            +     db.find(id).or_else(|| {
                            +         tracing::warn!(id, "user not found");
                            +         None
                            +     })
                              }
```

| Key | Action |
|---|---|
| `]h` / `[h` | Jump to next / previous hunk |
| `a` | Accept current hunk (stage for write) |
| `r` | Reject current hunk (discard change) |
| `A` | Accept all hunks in current file |
| `R` | Reject all hunks in current file |
| `f` | Finalize — write all accepted hunks to disk |
| `q` | Dismiss — discard all proposals (no disk write) |
| `↑↓` / `j k` | Scroll within current hunk |
| `PageUp/Down` | Scroll by page |

The status bar shows `[accepted/total hunks]` during review.

---

### 10. Git commit after agent edits

After accepting and finalizing agent changes:

1. Switch to git panel — `Alt+G`
2. All modified files appear under **Unstaged** (M/A/D/R)
3. Press `s` on a file (or `S` for all) to stage it
4. Type a commit message in the input field
5. Press `Enter` to commit

Or use the embedded terminal (`Ctrl+J`):
```bash
git add -p        # interactive staging
git commit -m "fix: invalidate tokens on logout"
```

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
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close tab |
| `Alt+[` / `Alt+]` | Previous / next tab |
| `Ctrl+F` | Find in buffer |

### Focus

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
| `Alt+C` | Changes (git working tree) |

### Side panel mode

| Key | Mode |
|---|---|
| `Alt+A` | Agent chat |
| `Alt+W` | Swarm dashboard |
| `Alt+G` | Git panel |

### Layout presets

| Key | Layout |
|---|---|
| `Alt+5` | Standard (tree 15% / editor 70% / side 15%) |
| `Alt+6` | Editor focus (tree 15% / editor 85%) |
| `Alt+7` | Full editor (editor 100%) |
| `Alt+8` | Code + notes (editor 60% / side 40%) |
| `Alt+9` | Wide tree (tree 25% / editor 75%) |
| `Alt+0` | Three columns (tree 20% / editor 55% / side 25%) |

### Editing

| Key | Action |
|---|---|
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+A` | Select all |
| `Ctrl+K` | Delete line |
| `Ctrl+D` | Duplicate line |
| `Alt+↑` / `Alt+↓` | Move line up / down |
| `Ctrl+←` / `Ctrl+→` | Move by word |
| `Shift+arrows` | Extend selection |
| `F5` | Format at current level |
| `F6` | Cycle format level |
| `Alt+P` | Toggle markdown preview |

---

## Slash commands

| Command | Action |
|---|---|
| `/model <name>` | Override model for this conversation (sonnet/opus/haiku) |
| `/effort <level>` | Override effort level (off/low/high) |
| `/namespace <name>` | Override write namespace |
| `/swarm <task>` | Run a multi-agent swarm immediately |
| `/cswarm <task>` | Coordinated swarm: Opus plans first, DSL file opens for review |
| `/run <file.gaviero>` | Compile and execute a workflow script |
| `/run <file.gaviero> <prompt>` | Run script with `{{PROMPT}}` substitution |
| `/undo-swarm` | Revert the last swarm run (git reset to pre-swarm SHA) |
| `/attach <path>` | Attach file to next message |
| `/detach <name>\|all` | Remove attachment |
| `/remember <text>` | Store text to persistent memory |
| `/help` | Show all commands |

---

## Swarm dashboard

Visible when a `/swarm` or `/cswarm` command is running (switch with `Alt+W`).

Shows:
- Agent table with status, tier, backend, and elapsed time per agent
- Per-agent activity log (tool calls, status changes, file writes)
- Phase indicator (planning → executing → merging)
- Cost estimate

Press `Enter` on a completed agent to view its proposed diff inline.

---

## File tree

Switch to focus with `Alt+1` (when in FileTree mode).

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate |
| `Enter` | Open file or toggle directory |
| `Space` | Toggle directory expand |
| `n` | New file |
| `N` | New folder |
| `r` | Rename |
| `d` / `Delete` | Delete |

---

## Memory

Gaviero maintains persistent semantic memory per project. Agents automatically read from and write to configured namespaces. You can store notes directly from chat:

```
/remember The auth module uses JWT with RS256 keys stored in .env
/remember All database queries go through the Repository trait — never use raw SQL
```

Memory persists across sessions and is scoped per workspace namespace.

---

## Session persistence

On exit, Gaviero saves:
- Open tabs and cursor/scroll positions
- Panel visibility and active modes
- File tree expanded directories
- Terminal split size
- Active layout preset
- Chat conversations (per workspace)

All restored on next launch.

---

## Configuration

Settings are read from `.gaviero/settings.toml` in the workspace root. Created automatically on first run. Key settings:

```toml
[agent]
namespace = "my-project"      # default write namespace
read_namespaces = ["shared"]  # additional read namespaces

[editor]
exclude_patterns = ["node_modules/", "target/", ".git/"]

[theme]
file = "themes/default.toml"  # colour theme path
```
