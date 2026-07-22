# gaviero-dsl

## Overview

Compiler for declarative `.gaviero` workflow scripts. Define multi-agent tasks with scopes, dependencies, verification, and iteration strategies; the compiler transforms DSL source into a `CompiledPlan` DAG consumed by the swarm runtime in [`gaviero-core`](../gaviero-core/README.md).

Pipeline: `include resolver → logos lexer → chumsky parser → AST → semantic analysis → CompiledPlan`. This crate does **not** execute anything.

## Installation

```bash
cargo build -p gaviero-dsl
cargo test  -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Usage

### From the CLI

```bash
gaviero-cli --script path/to/workflow.gaviero
gaviero-cli --script workflows/refactor.gaviero --var PLANS=output --param judge=claude:haiku@low
gaviero-cli --script examples/update_docs.gaviero \
    --tiers-file examples/profiles/doc-codex.gaviero
```

### From the TUI

```
/run path/to/workflow.gaviero
/run path/to/workflow.gaviero "runtime prompt text"
```

The runtime prompt substitutes for `{{PROMPT}}` placeholders.

### Coordinated planning

```bash
gaviero-cli --coordinated --task "refactor the auth module" --output tmp/plan.gaviero
gaviero-cli --script tmp/plan.gaviero
```

## Examples

### Two-agent workflow

```gaviero
client reasoning { tier expensive model "claude:sonnet" }
client local_exec  { tier cheap     model "ollama:qwen2.5-coder:7b" privacy local_only }

agent design {
    description "Plan the refactor"
    client reasoning
    scope { read_only ["src/" "tests/" "docs/"] }
    prompt "Inspect the implementation and produce a concrete refactor plan."
}

agent implement {
    description "Apply the refactor"
    client local_exec
    depends_on [design]
    scope { owned ["src/" "tests/"] read_only ["docs/"] impact_scope true }
    context { callers_of ["src/auth/session.rs"]  tests_for ["src/auth/"]  depth 2 }
    prompt "Implement the approved plan."
    max_retries 4
}

workflow refactor_auth {
    steps [design implement]
    strategy refine
    verify { compile true  clippy true  impact_tests true }
}
```

### Shipped examples

Ten workflow scripts and three tier profiles live in `examples/`:

| File | Demonstrates |
|---|---|
| `clients.gaviero` | Shared client pool |
| `plan_refinement.gaviero` | Dual-model plan refinement with judge loop |
| `generic_consensus.gaviero` | N-reviewer consensus via roster expansion |
| `phased_plan.gaviero` | Dynamic phase extraction |
| `codebase_review.gaviero` | Rolling replan/apply loop |
| `update_docs.gaviero` | Parallel doc rewrite with `--tiers-file` |
| `security_audit_memory.gaviero` | `memory {}` overrides, `read_ns`, staleness |

**Tier profiles** (`examples/profiles/`, `tier <role> <client>` lines only):

- `doc-claude.gaviero`
- `doc-codex.gaviero`
- `doc-cursor.gaviero`

```bash
gaviero-cli --script crates/gaviero-dsl/examples/plan_refinement.gaviero \
    --prompt "Add workspace settings cascade"
```

Scripts that `include "clients.gaviero"` must be compiled via `compile_file` or `gaviero-cli --script` — inline `compile()` rejects includes.

### Model strings

Canonical form is `provider:model`. Bare names are rejected at dispatch.

| Provider | Examples |
|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus` |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` |
| Cursor | `cursor:claude-4-sonnet` |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:model-name` |
| DeepSeek | `deepseek:deepseek-v4-pro`, `deepseek:deepseek-v4-flash` |

## Configuration

The DSL has no runtime config — behaviour is declared in the script and adjusted at compile time:

| Override | CLI flag | Precedence |
|---|---|---|
| Vars | `--var KEY=VALUE` | agent `vars {}` > CLI > script `vars {}` |
| Tiers | `--tiers-file` | CLI > script/includes |
| Workflow params | `--param NAME=VALUE` | Required params without defaults must be supplied |

Provider resolution, memory namespaces, and verification execution are configured via `.gaviero/settings.json` on the `gaviero-core` side. See [gaviero-cli](../gaviero-cli/README.md#configuration).

## API

### Compile entry points

```rust
use gaviero_dsl::{
    compile, compile_with_vars, compile_file,
    workflow_execution_mode, load_tier_overrides,
    CompiledPlan, ExecutionMode,
};

// Inline (no `include` statements)
let plan = compile(source, filename, Some("workflow_name"), Some("runtime prompt"))?;

// Inline + CLI overrides
let plan = compile_with_vars(
    source, filename, Some("workflow_name"), Some("runtime prompt"),
    &override_vars, &override_tiers, &override_params,
)?;

// From disk — resolves `include` transitively
let plan = compile_file(path, Some("workflow"), Some("prompt"), &[], &[], &[])?;

let mode: ExecutionMode = workflow_execution_mode(path, Some("my-workflow"))?;
let tiers = load_tier_overrides(path_to_profile)?;
```

All compile functions return `Result<CompiledPlan, miette::Report>` with source diagnostics on failure.

`CompiledPlan` is re-exported from `gaviero_core::swarm::plan`. Public modules: `ast`, `compiler`, `reviewers`, `workflow_params`, `error`, `lexer`, `parser`, `resolver`, `tiers`.

### Language surface (summary)

- **Client** — model + tier + effort + optional `extra {}` provider pairs
- **Agent** — scope, prompt, dependencies, `context {}`, `memory {}`, `tools []`
- **Workflow** — steps, `verify {}`, iteration strategy, explicit `loop {}` blocks
- **Include** — splice shared declarations; cycles rejected; requires `compile_file`
- **Scope** — `owned`, `read_only`, `impact_scope` with glob patterns
- **Params** — typed `param` on workflows; roster `id=provider:model[@effort],…`

Full language reference: see the prior detailed blocks in [ARCHITECTURE.md](ARCHITECTURE.md) or browse `examples/`.

## See Also

- [gaviero-core](../gaviero-core/README.md) — execution runtime
- [gaviero-cli](../gaviero-cli/README.md) — `--script`, `--var`, `--param`, `--tiers-file`
- [Root ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
