# gaviero-tui

Full-screen terminal editor. Rendering + input only — all logic delegates to `gaviero-core`.

Binary: `gaviero` ([src/main.rs](src/main.rs)).

## Build & Test

```bash
cargo test -p gaviero-tui
cargo clippy -p gaviero-tui
cargo run -p gaviero-tui        # launch editor in current dir
cargo run -p gaviero-tui -- path/to/repo
cargo run -p gaviero-tui -- name.gaviero-workspace
cargo run -p gaviero-tui -- --workspace path/to/dir   # multi-folder from a directory
```

Workspace dispatch: directory → `Workspace::single_folder`; `*.gaviero-workspace` → `Workspace::load`; `--workspace <dir>` → the `*.gaviero-workspace` already in `<dir>`, else one the setup screen writes ([src/main.rs](src/main.rs)). A bare directory that holds a `*.gaviero-workspace` first gets `setup::ask_folder_launch` (workspace preselected, Esc = plain folder; no TTY = plain folder + log warning; `--folder` skips it). Under the supervisor the answer is recorded in the `GAVIERO_LAUNCH_STATE` file so a restart onto a new build never stops at the prompt ([src/supervisor.rs](src/supervisor.rs)).

**First-run setup** ([src/setup.rs](src/setup.rs)) runs *before* the editor, owning its own raw-mode session so the C1 consent prompt after it still reads plain stdin. Triggers at every launch while the target has no `.gaviero/settings.json` — the folder's own (`LaunchMode::Folder`) or the workspace layer beside an opened `.gaviero-workspace` file (`LaunchMode::OpenWorkspace`, also reached by `--workspace` finding one) — or, for `--workspace` (`LaunchMode::Workspace`), no `*.gaviero-workspace` yet; never without a TTY. Every mode is one screen writing that single file: editor-only (preselected, also Esc; writes nothing), minimal (`{}`), or full — a byte-for-byte copy of the global settings file (warns when it carries `remote.relay.secret`), else `builtin_full_settings`. Only the highlighted option's description is drawn so the three titles fit a short pane. `--workspace` picks the member folders first; its editor-only still writes the workspace file. Esc anywhere opens editor-only and writes nothing (the opened workspace file, else the directory as a single folder). Existing files are never overwritten.

**Editor-only session:** `App::enter_editor_only` (before the event loop) empties `graph_services` so nothing lazily creates `code_graph.db`; `main` skips `start_agent_services` (Cursor hooks, egress, memory → MCP, code graph, remote), the `.cursorignore` sync and the C1 probe. `App::enforce_editor_only` runs after every event and after `restore_session`: side panel hidden, focus never `SidePanel`, left panel only explorer / FIND. `controller::editor_only_blocks` refuses side-panel, CHANGES, graph and agent actions with a status message. Slash commands are unreachable (they live in the chat input).

## Architecture

- [`app.rs`](src/app.rs) + [`app/`](src/app) — `App`, layout, focus, observers, chat-memory ([`app/chat_memory.rs`](src/app/chat_memory.rs)), topology cache ([`app/session.rs`](src/app/session.rs)), slash commands ([`app/commands.rs`](src/app/commands.rs)), render ([`app/render.rs`](src/app/render.rs)).
- [`event.rs`](src/event.rs) — crossterm / notify / tick / observer events; Windows paste-burst coalescer.
- [`keymap.rs`](src/keymap.rs) — Ctrl = editor, Alt = workspace. **F7** = word-wrap primary (`Alt+Shift+Z` fallback — NVIDIA steals plain `Alt+Z`). `Ctrl+Up/Down` when hosts steal `Alt+arrows`. `Ctrl+Alt+Left/Right` resize explorer/editor/side widths (`Alt+Left/Right` reserved for tmux/psmux).
- [`platform.rs`](src/platform.rs) — all platform quirks (ConPTY mouse, AltGr, Ctrl+C forwarder). New quirks go here, not inline.
- [`setup.rs`](src/setup.rs) — pre-TUI first-run screen; editor-only, or writes `.gaviero/settings.json` (`{}`, or a copy of the global file / built-in full settings) and, with `--workspace`, the `.gaviero-workspace` file. Runs before `App` exists, so it holds no `App` state.
- [`editor/`](src/editor) — buffer, view, highlight, markdown, diff overlay, LCS diff, wrap, [`fold`](src/editor/fold.rs).
- [`panels/`](src/panels) — file tree, agent chat, history ([`history_panel.rs`](src/panels/history_panel.rs), `Alt+h`), git, terminal, search, memory, status bar.
- [`widgets/`](src/widgets), [`theme.rs`](src/theme.rs).

**Observer bridge:** implements `WriteGateObserver` and `AcpObserver` from [`gaviero_core::observer`](../gaviero-core/src/observer.rs), plus the memory / manifest / MCP observers. Each holds an event-channel sender. **No background task mutates `App` directly.**

**SWARM panel** (`Alt+w`): [`panels/swarm_dashboard.rs`](src/panels/swarm_dashboard.rs) holds state and rendering, [`app/swarm_run.rs`](src/app/swarm_run.rs) launching and input. Three views: script picker (`ScriptCatalog`, plus a path typed in the filter), input form (`gaviero_dsl::inspect_file`, plus a Profile selector when any model profile exists; its Prompt field is the chat's box, [`widgets/prompt_input.rs`](src/widgets/prompt_input.rs), and `$skill` is expanded at launch; `file` and `dir` vars complete like `@`, and roster / client params complete `tier:<name>` from the selected profile plus the script), and the run (one row per planned agent; the selected agent's output below). A run is `pipeline::execute` in-process, prepared by the same [`script_run`](../gaviero-core/src/swarm/script_run.rs) helpers as `gaviero-cli --script`; keep the two in step. It uses the pipeline's Write Gate, not TURN REVIEW. Every update carries a run id so a stopped run's stragglers are dropped. This panel is the only way the TUI runs a script: chat has no `/run`, and `/swarm`, `/cswarm` and `/undo-swarm` stay gone. Remote does not mirror the panel.

**Turn review (mandatory):** [`app/turn_review.rs`](src/app/turn_review.rs) + the dispatch task in [`app/side_panel.rs`](src/app/side_panel.rs). Each chat turn is bracketed by `app.turn_capture.begin/end` (core `turn_capture`); the session runs with `AgentOptions::host_capture` and the conversation's gate mode `AutoAccept`. A non-empty change set opens the TURN REVIEW left panel (non-modal) and blocks *that* conversation's next prompt (desktop keeps the typed prompt; remote gets `turn_review_pending`). Pending reviews and their decisions persist under `.gaviero/turns/pending/` and reopen on startup. Any host write made while a turn may be running — editor saves, review reverts, MCP config synthesis — must go through `turn_review::note_host_write` or it is attributed to the agent.

**History capture:** one `gaviero_core::history::HistoryRecorder` per `App` (`app.history`). Every capture point keys by `turn_id`, never by conversation — `is_streaming` clears on the final message, before the turn's task finishes, so the next turn can start first. The HISTORY panel reads the NDJSON file; nothing reads through the recorder.

**Restart on new build:** [`supervisor.rs`](src/supervisor.rs) + [`app/restart.rs`](src/app/restart.rs). The `gaviero` a user starts (on a TTY, unless `GAVIERO_NO_SUPERVISOR` is set) is a thin parent that runs the real TUI as a child with `GAVIERO_SUPERVISED=1` and relaunches it in the same pane whenever it exits with code 75. The child stats its own exe every 2s and decides once when a changed exe has settled: if a task is running (agent turn, background agent, terminal command) or it is unsupervised, it keeps running the old build and shows a "new version installed — restart manually" popup (any key dismisses), with no automatic restart afterwards. Otherwise it waits out transient blockers (unsaved buffer, review, dialog, input in the last 15s), then quits through the normal save path and exits 75. The session host gets the same: `gaviero --sessions` is supervised by `supervisor::supervise_host` (the child carries `GAVIERO_SESSION_HOST_RESTART`, a private record file), and [`session_host/restart.rs`](src/session_host/restart.rs) waits until no tab reports work, no host dialog is open and no host input for 15s, writes the record (tab paths in order, the current tab, each tab's launch-state file verbatim), quits like Ctrl+Q and exits 75; the next host reopens from the record (consumed once) instead of its arguments. Its tabs carry `GAVIERO_SESSION_HOST_RESTARTS` and skip their own build watch, so a busy tab never gets the "restart manually" popup — the host restarts it once idle. An unsupervised host only says once on the tab bar that a restart by hand is needed. [`scripts/deploy.ps1`](../../scripts/deploy.ps1) installs over running binaries by renaming them to `*.exe.old-<ts>` (Windows allows renaming a running exe, not deleting it).

**Session host** (`gaviero --sessions [path]`, [`session_host/`](src/session_host)): a separate program in the same binary, dispatched first thing in `main()` (before the input probe and the supervisor; refused inside a session). It runs every session as an unmodified `gaviero <path>` child in a pseudo-terminal (ConPTY on Windows) with `GAVIERO_SUPERVISED` + `GAVIERO_LAUNCH_STATE` (it supervises each tab: exit 75 relaunches in place, with the supervisor's `MIN_RUN` / `RESTART_BACKOFF`), `GAVIERO_SESSION_HOST` and `GAVIERO_SESSION_CONTROL`, emulates each screen with `vt100`, and draws the active one under a one-row tab bar. Modules: [`encode`](src/session_host/encode.rs) (std only — win32-input-mode key sequences, SGR mouse, DSR/DA1 answers; ConPTY itself asks DSR at startup), [`session`](src/session_host/session.rs) (child, reader/writer threads, status), [`input_windows`](src/session_host/input_windows.rs) (`ReadConsoleInputW` records forwarded as win32-input-mode, so AltGr / key-up survive) / [`input_term`](src/session_host/input_term.rs) (Unix: crossterm events re-encoded; compiled, not live-tested), [`render`](src/session_host/render.rs), [`picker`](src/session_host/picker.rs) (Alt+N; a whole-disk folder browser — empty box = recents + start folder + home + `platform::filesystem_roots` (drives via `GetLogicalDrives`), Left = parent, Right = enter; completion from [`widgets/path_listing.rs`](src/widgets/path_listing.rs)), [`state`](src/session_host/state.rs) (`session-host.json` under the data dir — recents + open set; `GAVIERO_SESSION_HOST_STATE` overrides). Host keys: Alt+Left/Right, Alt+Shift+Left/Right (move the current tab one place, no wrap; `HostState::reorder_open` saves the order — Windows Terminal 1.24 passes these through with one pane despite its default Resize Pane binding), Alt+N, Alt+Q (close the active tab; the previous one becomes current), Ctrl+Q (quit the host, open set kept — taken by the host, never forwarded; Ctrl without Alt so AltGr+Q stays text). Alt+X is never bound: Windows intercepts it, click on a tab (its leading `x` — `TabBar::closable` / `close_at_x` — closes it); everything else goes to the active session. A child reports status in its title, OSC 2 `gaviero-session:<idle|busy|attention>[:work,…]` (`App::session_host_status`, written by `main` only under the host; work = `restart::work_in_progress` keywords `agent`, `swarm`, `terminal`, `unsaved`, separated by `:` because terminals split OSC parameters on `;`). The host reads it as `session::Activity` (`waiting` = `attention`, `agent` = `agent`/`swarm`, `shell` = `terminal`) and draws two circles after every tab's label, the current one included (`render::badges` → `TabBar::badges`, fixed slots so widths never change): agent running / waiting / finished, shell running / finished. "Finished" is latched in `on_output` only when the work ends while the tab is not current (`agent_done_unseen` / `shell_done_unseen`) and cleared by `switch_to` on the tab being left (the tab made current keeps its circles), or in `on_output` when the current tab's activity changes. Closing a tab or quitting the host with work in progress opens the close guard (`Confirm`, a centred popup listing it per tab; only `y` proceeds); with none it acts at once. The host controls a child through its control file, which `App::poll_session_control` consumes on Tick: `exit` quits at once through the normal exit path (session state saved, no question), `quit` goes through `try_quit`. Closing a tab removes it immediately and writes `exit`; quitting the host writes `exit` to every child. Either way a child still running after `EXIT_GRACE` (3 s) is killed, and the host waits for closing children before it exits. Test-only: `GAVIERO_INPUT_PROBE` turns the binary into an input probe ([`probe`](src/session_host/probe.rs)) for `tests/session_host_*.rs`. Host log: `gaviero-sessions.log` beside `gaviero.log`.

**Remote sidecar:** every TUI launch with `remote.relay.host` set starts a background WSS sidecar (`gaviero-remote`, Plan C) unless `remote.enabled` is false. gaviero-relay is the only transport: the listener binds loopback, the pinned-CA leaf, the bind, the relay client and the machine registry are all set up off the event loop (`Event::RemoteStarted` / `Event::RemoteUnavailable`). The phone mirrors every conversation tab and can decide turn reviews per file (protocol 1.2 `turn_review`; per-hunk and the drift confirmation stay desktop-only). Pairing is `/remote` (QR + machine token). See [`crates/gaviero-remote/PROTOCOL.md`](../gaviero-remote/PROTOCOL.md).

**Authoritative slash list:** [`app/commands.rs`](src/app/commands.rs) (and chat helpers in [`panels/agent_chat.rs`](src/panels/agent_chat.rs)). Groups: session (`/model`, `/effort`, `/autoapprove`/`/yolo`, …), context (`/lite`, `/inject`, `/context mode …`), memory, MCP (`/mcp`, `/mcp probe` — probe runs from `gaviero-cli --mcp-reach-probe`), skills (`/skills`, `$skill`). Do not maintain a second inventory in ARCHITECTURE.md — point here.

## Conventions

- **Single event channel.** All external sources funnel into one `mpsc::unbounded_channel<Event>`.
- **Event-loop golden rule.** `draw → recv → handle → repeat`. Render is pure; mutation only in `handle`.
- **No `Mutex` in the TUI.** State changes go through the event loop.
- Diff overlay: `]h`/`[h` navigate; `a`/`r` accept/reject; `A`/`R` all; `f` finalize; `q` exit.
- TURN REVIEW has exactly four decisions, each applied immediately: `a` / `r` accept / reject the selected file, `A` / `R` accept / reject every file not decided yet. Reject = back to the pre-prompt version. The review ends when every file is decided. The list always groups files under a header per workspace folder (`FileChange::root`); selection, `j`/`k`, the next-undecided jump and clicks follow that drawn order. Rejecting a file changed since the turn asks for the same key again. Navigation only otherwise: `j`/`k` file, `J`/`K` scroll the diff, `Tab` next pending review. HISTORY → FILES (`6`): `u` twice undoes a reviewed turn (last 5).
- Merge conflicts: F8/F9 next/previous region; save stages when markers are gone ([`gaviero_core::git_conflict`](../gaviero-core/src/git_conflict.rs)).
- Wrapped-layout editing receives viewport width via [`app/editing.rs`](src/app/editing.rs); never compute visual position outside the editor module.
- **`WrapLayout` is the only line↔row map.** Folds hide lines by omitting their segments, so anything converting between file lines and screen rows goes through the layout or `Buffer::hidden_lines()` — never `cursor.line == scroll.top_line` arithmetic ([`editor/fold.rs`](src/editor/fold.rs)).

## Rules

- **Never call core APIs from a panel render path.** Panels read `App` state; mutation goes through the event loop.
- **Never hold a lock across `.await` on the UI side.** Prefer a channel.
- **Topology prefetch is best-effort.** UI must render before the cache resolves ([`app/session.rs`](src/app/session.rs)).
- **Slash-command parsing is line-prefix only.** Use `strip_prefix("/cmd")` ([`app/commands.rs`](src/app/commands.rs)); no regex.

## Dependencies

- `ratatui 0.30` + `crossterm 0.29` — rendering, input.
- `ropey 1.6` — rope buffer.
- `notify 7` — filesystem watcher.
- `portable-pty 0.9` + `vt100 0.16` — embedded terminal.
- `arboard 3` + `base64` + `png` — clipboard / image paste.
- `unicode-width 0.2` — visual width for wrap.
- `windows-sys 0.59` (Windows only) — Ctrl+C forwarder ([`platform.rs`](src/platform.rs)).
- `embed-manifest 1.4` (build) — Windows application manifest.
- `gaviero-core`, `gaviero-dsl` — runtime + DSL compilation.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — event loop, layout, panel patterns, observer bridge.
- [README.md](README.md) — keybindings, settings cascade, themes.
- [../../CLAUDE.md](../../CLAUDE.md) — workspace rules; **`plans/` + `research/` are history, not spec** (a newer prompt may contradict them and wins).
