# gaviero-tui

`gaviero-tui` is the full-screen terminal workspace for Gaviero. It combines a
multi-tab editor, file tree, search, review flows, agent chat, swarm dashboard,
git panel, and embedded terminal in one process.

This crate is the interactive front-end. Runtime behavior still lives in
`gaviero-core`.

## What the TUI includes

- File tree, search, review, and git changes panels
- Multi-buffer editor with syntax highlighting and markdown preview
- Agent chat with streaming output, file references, and attachments
- Swarm dashboard for `/swarm`, `/cswarm`, and `.gaviero` runs
- Embedded terminal backed by `gaviero-core::terminal`
- Session restore for tabs, layout, and conversations

## Provider-aware agent usage

Chat and swarm commands share the same provider model-spec rules as the core
runtime.

- `sonnet`, `opus`, `haiku`: Claude models
- `ollama:<model>` or `local:<model>`: local Ollama models
- `agent.ollamaBaseUrl`: workspace setting for the Ollama server URL

Inside chat, `/model` can switch the active model. Example:

```text
/model ollama:qwen2.5-coder:7b
```

## Common commands

- `/model <name>`: set the active chat model
- `/run <file.gaviero> [prompt]`: compile and run a DSL workflow
- `/swarm <task>`: execute an immediate multi-agent swarm
- `/cswarm <task>`: generate a reviewable coordinated plan
- `/undo-swarm`: revert the last coordinated swarm result
- `/remember <text>`: store a memory entry
- `/attach <path>` / `/detach <name|all>`: manage attachments

## Workspace settings

The TUI reads `.gaviero/settings.json` through `gaviero-core::workspace`.
Relevant AI settings include:

```json
{
  "agent": {
    "model": "sonnet",
    "effort": "off",
    "maxTokens": 16384,
    "ollamaBaseUrl": "http://localhost:11434",
    "coordinator": {
      "model": "sonnet"
    }
  }
}
```

The first-run dialog can create a default settings file for a new workspace.

## Running

```bash
gaviero
gaviero /path/to/repo
gaviero /path/to/workspace.gaviero-workspace
```

Logs are written to the platform cache directory because stderr is not visible
in the alternate-screen TUI session.

## Architectural shape

The large historical `app.rs` god object has been decomposed into `src/app/`
modules for controller, render, left-panel flows, side-panel flows, editing,
session restore, observers, and shared state. `src/panels/` remains the home of
panel state and panel rendering primitives.

See `ARCHITECTURE.md` for the module map and event flow.
