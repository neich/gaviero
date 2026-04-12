# gaviero-core

`gaviero-core` is the runtime library behind the Gaviero workspace. It owns
provider dispatch, chat and swarm execution, write-gated file proposals,
iteration and validation, workspace settings, memory, repo context building,
git/worktree orchestration, session persistence, and terminal management.

`gaviero-cli`, `gaviero-tui`, and `gaviero-dsl` are front-ends around this
crate. If behavior differs between the binaries, the integration layer is the
first place to look; the execution engine lives here.

## What this crate provides

- A provider-aware backend layer behind `swarm::backend::AgentBackend`
- A chat pipeline in `acp::client::AcpPipeline`
- Multi-agent orchestration in `swarm::pipeline`
- Tier routing and privacy routing in `swarm::router`
- Iterative retry/escalation logic in `iteration`
- Write proposal review/apply flow in `write_gate`
- Syntax and compile-time validation gates in `validation_gate`
- Workspace settings, namespaces, and session persistence
- Semantic memory, repo-map ranking, and code graph context
- Git repository/worktree helpers and embedded terminal management

## Provider model specs

The runtime now uses one model-spec convention across chat and swarm paths.

- `sonnet`, `opus`, `haiku`: Claude models via the Claude CLI backend
- `claude:<name>` or `claude-code:<name>`: explicit Claude model selection
- `ollama:<model>` or `local:<model>`: local Ollama model selection
- Unprefixed model names default to Claude for backward compatibility

Ollama base URLs are passed through `SwarmConfig.ollama_base_url` or the
workspace setting `agent.ollamaBaseUrl`.

## Main entry points

| Area | Entry points |
| --- | --- |
| Chat execution | `acp::client::AcpPipeline` |
| Swarm execution | `swarm::pipeline::execute()` |
| Coordinated planning | `swarm::pipeline::plan_coordinated()` |
| Iterative execution | `iteration::IterationEngine` |
| Backend abstraction | `swarm::backend::{AgentBackend, CompletionRequest}` |
| Tier routing | `swarm::router::TierRouter` |
| Write review/apply | `write_gate::WriteGatePipeline` |
| Validation | `validation_gate::ValidationPipeline` |
| Workspace settings | `workspace::Workspace` |
| Semantic memory | `memory::MemoryStore` |
| Repo ranking / graph | `repo_map::{RepoMap, graph_builder}` |
| Git/worktrees | `git::{GitRepo, WorktreeManager, GitCoordinator}` |
| Session / terminal | `session_state`, `terminal::TerminalManager` |

## Execution surfaces

### Chat

`AcpPipeline` enriches prompts with conversation history and file references,
then routes the request through the provider-aware backend layer. Claude models
still use the ACP subprocess path for the existing Claude CLI UX. Local
`ollama:` and `local:` models go through the shared backend executor.

### Swarm

`swarm::pipeline::execute()` runs compiled plans. It validates scopes, computes
dependency tiers, resolves a backend for each work unit through `TierRouter`,
runs the iteration engine, applies verification, and merges results when
worktrees are enabled.

### Coordinated planning

`swarm::pipeline::plan_coordinated()` asks the coordinator to produce a
reviewable `.gaviero` plan. The plan is meant to be inspected, optionally
edited, compiled by `gaviero-dsl`, and then executed with the normal swarm
pipeline.

## Module overview

- `acp`: Claude subprocess protocol plus the provider-aware chat pipeline
- `swarm`: planning, routing, execution, merge, verification, and backends
- `iteration`: retry/refine/best-of-N strategy logic
- `validation_gate`: post-edit validation gates
- `write_gate`: proposal review and file application
- `workspace`: settings cascade and namespace resolution
- `memory`: scoped semantic memory and consolidation
- `repo_map`: ranked context planning and knowledge graph storage
- `git`: git repository helpers and worktree orchestration
- `session_state`: persisted editor state for the TUI
- `terminal`: PTY lifecycle and terminal session helpers
- `tree_sitter`, `diff_engine`, `indent`, `scope_enforcer`: editor/runtime
  primitives used by higher-level systems

## Notes

- This crate is the source of truth for runtime behavior.
- The public Rust API is usable internally across the workspace, but it is not
  yet documented as a stable external SDK.
- If you are looking for language syntax, see `crates/gaviero-dsl`.
- If you are looking for UI composition, see `crates/gaviero-tui`.
