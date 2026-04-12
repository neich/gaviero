# gaviero-cli - Architecture

`gaviero-cli` is intentionally thin. It parses arguments, builds or compiles a
plan, wires observers, and delegates all runtime behavior to `gaviero-core`.

## File layout

```text
gaviero-cli/src/
└── main.rs
```

There is no separate domain layer in this crate by design.

## Responsibilities

- Parse command-line flags with `clap`
- Resolve the workspace root and settings
- Build one of the supported input-plan shapes
- Apply iteration overrides from CLI flags
- Optionally generate a coordinated `.gaviero` plan
- Run the swarm pipeline and print results
- Bridge `AcpObserver` and `SwarmObserver` events to stderr

## Execution flow

```text
parse args
  -> load Workspace
  -> init memory store (best effort)
  -> choose one input mode
       - --task       => synthetic WorkUnit
       - --work-units => deserialize JSON
       - --script     => gaviero_dsl::compile(...)
  -> apply CLI iteration overrides
  -> if --coordinated
       -> swarm::pipeline::plan_coordinated()
       -> write .gaviero plan and exit
     else
       -> swarm::pipeline::execute()
       -> print SwarmResult
```

## Input-mode differences

### `--task`

Creates one synthetic work unit and uses the resolved execution model spec to
populate `WorkUnit.model`.

### `--script`

Compiles a `.gaviero` file and leaves model/provider selection to the script
contents. This is the path that supports provider-aware `ollama:` or `local:`
model strings today.

### `--work-units`

Accepts raw runtime data. This is mainly for tests, automation, or callers that
already know the `gaviero-core` model.

## Observer bridge

The CLI defines two local observer implementations:

- `CliAcpObserver`: streams agent output and tool/validation events to stderr
- `CliSwarmObserver`: prints swarm phases, dispatches, costs, and completion

The binary does not interpret execution semantics itself. It only formats
runtime events for a shell session.

## Model and provider boundary

The CLI still does not implement provider logic. It only chooses:

- an execution model spec for synthetic/default runtime use
- an optional coordinator model spec for `--coordinated`
- or a compiled plan whose models are already embedded

Accepted model-spec syntax is validated through
`gaviero_core::swarm::backend::shared::validate_model_spec()`, and actual
provider routing remains in `gaviero-core`.

## Design intent

- Keep the binary auditable and easy to change
- Avoid duplicating runtime rules from `gaviero-core`
- Preserve clean stdout/stderr separation for automation
- Let DSL scripts carry richer provider and workflow configuration than the CLI
  flag surface
