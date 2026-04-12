# gaviero-tui - Architecture

`gaviero-tui` is a terminal UI shell around `gaviero-core`. It owns rendering,
event routing, editor interactions, and command orchestration. It does not own
provider logic, swarm semantics, validation rules, or write-gate behavior.

## Module layout

```text
gaviero-tui/src/
├── main.rs
├── app.rs
├── app/
│   ├── controller.rs
│   ├── layout.rs
│   ├── render.rs
│   ├── left_panel.rs
│   ├── review.rs
│   ├── side_panel.rs
│   ├── commands.rs
│   ├── editing.rs
│   ├── session.rs
│   ├── state.rs
│   └── observers.rs
├── editor/
├── panels/
├── widgets/
├── event.rs
├── keymap.rs
└── theme.rs
```

## Current decomposition

The TUI no longer treats `app.rs` as the whole application. The current split is:

- `app.rs`: integration shell and shared app struct
- `app/controller.rs`: top-level event handling and action dispatch
- `app/render.rs`: layout and drawing orchestration
- `app/left_panel.rs`: explorer/search/review/changes behavior
- `app/review.rs`: review and diff acceptance flows
- `app/side_panel.rs`: chat/swarm/git side-panel behavior
- `app/commands.rs`: slash-command handlers such as `/run`, `/swarm`, `/cswarm`
- `app/editing.rs`: editor and find-bar interactions
- `app/session.rs`: session restore/save integration
- `app/state.rs`: enums and state structs shared by the shell/modules
- `app/observers.rs`: bridges from `gaviero-core` observer callbacks into TUI
  events

Panel-specific state and rendering primitives stay under `panels/`. The `app/*`
modules own orchestration logic; the panel modules stay closer to view/state.

## Event architecture

Everything converges on one event loop in `main.rs`.

```text
background producers
  -> Event
  -> App::handle_event(...)
  -> App::render(...)
```

Main producers:

- crossterm keyboard/mouse input
- filesystem watcher
- tick timer
- terminal bridge events
- chat/swarm observer events
- memory initialization completion

All UI state mutation happens on the main TUI thread. Background tasks send
events and never mutate the `App` state directly.

## Rendering model

`app/render.rs` computes layout and delegates concrete drawing to editor and
panel modules.

- Tab bar and status bar are always rendered
- Left panel, editor, side panel, and terminal areas are computed from current
  visibility/layout state
- Fullscreen mode reuses the same title/content helpers rather than duplicating
  panel-specific chrome

The renderer reads state and invokes draw helpers. Behavioral decisions belong
in controller modules instead of render code.

## Command flow

The side panel owns chat input, but slash commands are delegated into
`app/commands.rs`.

Examples:

- `/run` compiles a `.gaviero` file and starts swarm execution
- `/swarm` runs a natural-language swarm directly
- `/cswarm` runs coordinated planning and produces a reviewable `.gaviero` plan
- `/remember` writes to memory
- `/attach` and `/detach` manage chat attachments

Those commands call into `gaviero-core` and update panel state via events and
observer callbacks.

## Provider-aware chat and swarm

The TUI does not special-case providers in the UI layer beyond model selection.

- Chat uses `gaviero_core::acp::client::AcpPipeline`
- Swarm commands call `gaviero_core::swarm::pipeline`
- Both paths share the same provider model-spec rules
- `agent.ollamaBaseUrl` is loaded from workspace settings and passed down

This keeps model selection, prompt enrichment, and backend routing consistent
across chat and swarm.

## Session and persistence

Session restore/save is intentionally isolated:

- UI session state goes through `gaviero_core::session_state`
- terminal sessions go through `gaviero_core::terminal`
- workspace settings go through `gaviero_core::workspace`

The TUI coordinates these systems but does not reimplement them locally.

## Design intent

- Keep runtime behavior in `gaviero-core`
- Keep rendering, routing, and session glue in the TUI
- Keep panel rendering/state separate from top-level controllers
- Keep observer-to-event translation explicit and testable
