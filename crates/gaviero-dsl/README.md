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
- **Agent** — scope, prompt, dependencies, `produces []`, `context {}`, `memory {}`, `tools []`
- **Workflow** — steps, `verify {}`, iteration strategy, explicit `loop {}` blocks
- **Include** — splice shared declarations; cycles rejected; requires `compile_file`
- **Scope** — `owned`, `read_only`, `impact_scope` with glob patterns
- **Produces** — exact artefact paths the agent must have written when its turn
  ends; enforced by the runtime, not by the prompt

### Output contracts (`produces`)

```
agent reviewer-refine {
    scope { owned ["{{OUT_DIR}}/{{REVIEWER_ID}}-summary-v*.md"] }
    produces ["{{OUT_DIR}}/{{REVIEWER_ID}}-summary-v{{ITER}}.md"]
}
```

Unlike `owned` these are literal paths, not globs. Compile-time vars are
substituted as usual; `{{ITER}}` / `{{PREV_ITER}}` survive to the runtime and
are substituted per loop pass, so one declaration covers every iteration.

Two checks enforce it, neither of which costs a model call:

- **Per agent** — an agent that ends its turn with a declared path missing or
  empty gets one corrective retry, then a `Failed` manifest. Without this,
  a model that narrates a file it never wrote, a proposal dropped by scope,
  and a genuine success are all indistinguishable `Completed` manifests.
- **Per loop iteration** — a loop refuses to invoke its judge unless every body
  agent delivered that pass, and aborts the run naming the agent and the missing
  paths. A judge scoring a panel that silently lost a member returns a
  meaningless verdict and burns the iteration budget. The check runs immediately
  before the judge is dispatched, so under `until … and …` a cheaper condition
  that fails first leaves it untouched.

Agents that declare no `produces` fall back to a weaker check: the loop
verifies that *something* the agent owns changed during the pass.

### Composed exit conditions (`until … and …`)

```
loop {
    agents [impl]
    until { compile true } and command "cargo test --quiet" and agent reviewer
}
```

A loop exits when **every** condition passes. Conditions are evaluated
cheapest-first — `verify` block, then `command`, then the judge agent — regardless
of the order they are written in, and evaluation stops at the first one that does
not pass. A judge is an LLM call, so it is only consulted once the deterministic
conditions already agree; a failing `cargo test` costs nothing but the test run.

At most one `agent` may appear: a loop produces a single verdict, so a second
judge has no defined precedence and is a compile error. `and` is a soft keyword —
a script that already uses `and` as a name keeps working.

A pass failed by a deterministic condition issues no judge verdict, so it leaves
the `irreconcilable_after` disagreement counter untouched, and the delivery gate
does not run — there is no panel to protect when no judge was dispatched.

### Termination (`timeout`, `irreconcilable_after`)

```
agent reviewer { timeout 3600 }              # per-dispatch budget, seconds
workflow w { steps [ loop { irreconcilable_after 2 } ] }
```

`timeout` bounds one dispatch of an agent — every retry included — and defaults
to 3600s. It is what makes a run finite: provider sessions only give up when
their subprocess *exits*, so a wedged-but-alive CLI otherwise hangs the workflow
with no upper bound. `timeout 0` opts out. A blown budget becomes a `Failed`
manifest, not an error, so the delivery gate and loop verdict handle it normally.
`gaviero-cli --run-timeout <secs>` adds an outer cap on the whole run.

`irreconcilable_after` is the failure-side mirror of `stability`: N consecutive
PASS verdicts mean the agreement is real, and N consecutive *identical* blocking
disagreements mean the deadlock is structural. Either that counter or an
explicit `"verdict": "irreconcilable"` from the judge stops the loop and writes
`consensus-irreconcilable.md` — one section per reviewer, from the judge's
`blockers` array. Repeat detection fingerprints the blockers' agent/criterion
pairing rather than the prose, so a judge rephrasing itself does not reset the
count. `0` disables it.
- **Params** — typed `param` on workflows; roster `id=provider:model[@effort],…`

Full language reference: see the prior detailed blocks in [ARCHITECTURE.md](ARCHITECTURE.md) or browse `examples/`.

## See Also

- [gaviero-core](../gaviero-core/README.md) — execution runtime
- [gaviero-cli](../gaviero-cli/README.md) — `--script`, `--var`, `--param`, `--tiers-file`
- [Root ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
