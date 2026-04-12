# gaviero-core - Architecture

`gaviero-core` is the shared execution layer for the workspace. The main
architecture rule is simple: front-end crates own input/output and stateful UI,
while this crate owns runtime behavior.

## Top-level structure

```text
gaviero-core/src/
├── acp/                 chat pipeline and Claude subprocess protocol
├── swarm/               planning, routing, execution, merge, verification
├── iteration/           retry/refine/best-of-N logic
├── validation_gate/     syntax and compile/test gates
├── write_gate.rs        proposal review/apply pipeline
├── workspace.rs         settings cascade and namespace resolution
├── memory/              scoped semantic memory and consolidation
├── repo_map/            ranked context planning and code graph storage
├── git.rs               git repo/worktree orchestration
├── session_state.rs     persisted editor session state
├── terminal/            PTY and shell session management
├── observer.rs          write/chat/swarm observer traits
└── types.rs             shared domain types
```

## Architectural boundaries

### 1. Provider abstraction

The provider boundary is `swarm::backend`.

- `backend/mod.rs` defines `AgentBackend`, `CompletionRequest`,
  `UnifiedStreamEvent`, `Capabilities`, and serializable `BackendConfig`.
- `backend/shared.rs` contains model-spec parsing, provider detection,
  prompt enrichment, and default system-prompt helpers.
- `backend/executor.rs` consumes normalized streams and either collects text or
  routes `<file>` blocks through the write gate.
- `backend/claude_code.rs` implements the Claude CLI backend.
- `backend/ollama.rs` implements the Ollama streaming backend.

This layer is shared by swarm execution and the provider-aware parts of chat.
That is the key parity guarantee: both flows now use the same request shape and
the same model-resolution rules.

### 2. Chat execution

The `acp` module exists for Claude CLI compatibility and chat UX.

- `acp/session.rs` manages Claude subprocess sessions and `AgentOptions`.
- `acp/protocol.rs` parses streaming ACP events.
- `acp/client.rs` builds prompts, resolves file refs/attachments, and then:
  - uses the shared backend path for `ollama:` / `local:` models
  - keeps the Claude ACP subprocess path for Claude-backed chat sessions

The important design point is that prompt enrichment and model selection are no
longer split across unrelated code paths.

### 3. Swarm execution

`swarm::pipeline` is the orchestration shell around several smaller systems.

- `plan.rs` defines `CompiledPlan`, verification config, and loop config
- `models.rs` defines `WorkUnit`, manifests, statuses, and results
- `router.rs` maps tier/privacy/model overrides to concrete backends
- `pipeline.rs` executes plans, tiers, loops, verification, and merges
- `coordinator.rs` plans multi-agent work and can emit `.gaviero` DSL
- `planner.rs` is the legacy natural-language planner
- `merge.rs` handles merge and conflict resolution
- `validation.rs` enforces scope rules and dependency ordering

Execution shape:

```text
CompiledPlan
  -> validate scopes
  -> compute dependency tiers
  -> resolve backend per work unit via TierRouter
  -> run IterationEngine for each unit
  -> run workflow verification
  -> merge branches/worktrees if enabled
  -> return SwarmResult
```

Single-agent plans use a fast path, but they still resolve through the same
router and iteration contracts as multi-agent runs.

### 4. Iteration and validation

`iteration::IterationEngine` owns retry strategy, best-of-N sampling, test-first
mode, and escalation timing. It depends on:

- a backend factory or backend resolver
- `validation_gate::ValidationPipeline`
- feedback loops from failed validation passes

This separation matters because the swarm pipeline decides *what* to run,
while the iteration layer decides *how repeated execution is managed*.

### 5. Write application and review

The runtime never writes arbitrary model output directly to disk. Proposed file
blocks flow through `write_gate::WriteGatePipeline`, which supports interactive,
auto-accept, deferred, and reject-all modes. Both chat and swarm use the same
proposal machinery.

### 6. Context and long-lived state

Several subsystems enrich prompts or preserve state across runs:

- `workspace.rs`: workspace roots, settings, namespaces, model defaults,
  `agent.ollamaBaseUrl`, coordinator/tier settings
- `memory/`: scoped memory storage and retrieval
- `repo_map/`: file ranking and graph-based impact/context expansion
- `session_state.rs`: TUI tabs, layout, conversation/session restoration
- `terminal/`: PTY sessions for the embedded terminal panel

These modules are intentionally independent from UI code so both the CLI and
the TUI can reuse the same state and configuration rules.

## Verification surfaces

There are two verification surfaces in the codebase:

- Workflow-level `verify { ... }` settings compiled from the DSL and executed
  directly in `swarm::pipeline`
- Strategy-specific verifier types and helpers under `swarm::verify`

The first is the everyday execution path. The second holds richer review/test
strategy machinery used by coordinator and review-oriented flows.

## Observability

`observer.rs` defines the runtime callback surface:

- `WriteGateObserver`
- `AcpObserver`
- `SwarmObserver`

Front-ends implement these traits and translate runtime events into stderr
logs, TUI events, or other reporting layers. `gaviero-core` itself stays
presentation-agnostic.

## Design intent

- Keep provider logic behind one backend abstraction.
- Keep planning/execution separate from UI and CLI concerns.
- Keep edit application gated and observable.
- Keep workspace settings and state reusable across all front-ends.
- Favor explicit module boundaries over large cross-cutting helpers.
