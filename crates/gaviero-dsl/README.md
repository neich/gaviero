# gaviero-dsl

Compiler for declarative `.gaviero` workflow scripts. Define multi-agent tasks with scopes, dependencies, verification, and iteration strategies; the compiler transforms DSL source into execution DAGs consumed by the swarm runtime.

## Overview

A `.gaviero` script declares **clients** (models), **agents** (units of work with file scopes and prompts), and **workflows** (orchestration with verification and iteration). The compiler pipeline is:

```
include resolver → logos lexer → chumsky parser → AST → semantic analysis → CompiledPlan DAG
```

This crate does *not* execute anything — runtime execution, provider model resolution, verification, and git operations all live in [`gaviero-core`](../gaviero-core/README.md). `gaviero-dsl` only lexes, parses, validates (scope overlaps, dependency cycles), and emits a `CompiledPlan`.

## Installation

```bash
cargo build -p gaviero-dsl
cargo test  -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Core Concepts

**Client** — a model plus tier, effort, and optional provider extras:
```gaviero
client reasoning { tier expensive model "claude:opus"   effort high default }
client fast      { tier cheap     model "claude:sonnet" effort low }
```

**Tier alias** — a re-pointable routing label bound to a client:
```gaviero
tier cheap     fast
tier expensive reasoning
```

**Vars** — script-level key/value substitutions applied across all agents:
```gaviero
vars { PLANS "plans"  VERSION "1" }
```

**Prompt** — named, reusable prompt templates with `{{VAR}}` substitution:
```gaviero
prompt review-body #"
    Review {{PLANS}}/{{MODEL}}-draft.md and list all issues.
"#
```

**Agent** — a unit of work with scope, prompt, dependencies, and optional blocks:
```gaviero
agent design { description "..." client reasoning scope { ... } prompt review-body }
```

**Workflow** — orchestrates agents with an execution strategy, verification, and loop rules:
```gaviero
workflow review_and_fix { steps [design fixer] verify { ... } }
```

## Usage

### From the CLI

```bash
gaviero-cli --script path/to/workflow.gaviero
gaviero-cli --script workflows/refactor.gaviero --var PLANS=output
```

### From the TUI editor

```
/run path/to/workflow.gaviero
/run path/to/workflow.gaviero "runtime prompt text"
```

The runtime prompt is substituted for `{{PROMPT}}` placeholders in agent prompts.

### Generate from coordinated planning

Let the coordinator write a plan for you, then inspect, edit, and run it:

```bash
gaviero-cli --coordinated --task "refactor the auth module" --output tmp/plan.gaviero
gaviero-cli --script tmp/plan.gaviero
```

## Examples

### A complete two-agent workflow

Create `refactor.gaviero`:

```gaviero
client reasoning {
    tier  expensive
    model "claude:sonnet"
}

client local_exec {
    tier    cheap
    model   "ollama:qwen2.5-coder:7b"
    privacy local_only
}

agent design {
    description "Plan the refactor"
    client reasoning
    scope { read_only ["src/" "tests/" "docs/"] }
    prompt #"
        Inspect the current implementation and produce a concrete refactor plan.
        Call out risks, missing tests, and file ownership.
    "#
}

agent implement {
    description "Apply the refactor"
    client local_exec
    depends_on [design]
    scope {
        owned        ["src/" "tests/"]
        read_only    ["docs/"]
        impact_scope true
    }
    context {
        callers_of ["src/auth/session.rs"]
        tests_for  ["src/auth/"]
        depth      2
    }
    prompt "Implement the approved plan and keep changes scoped."
    max_retries 4
}

workflow refactor_auth {
    steps [design implement]
    strategy refine
    verify { compile true  clippy true  impact_tests true }
}
```

### Example scripts in `examples/`

Several example workflows ship in `examples/`. They `include "clients.gaviero"` for the shared
client pool, so compile them via `compile_file` or `gaviero-cli --script` — inline `compile()`
rejects includes. A representative selection:

| File | Demonstrates |
|------|----------------|
| `clients.gaviero` | Shared profiles: `claude:opus`, `claude:sonnet`, `codex:gpt-5.5`, `codex:gpt-5.4` |
| `plan_refinement.gaviero` | Dual-model plan refinement; judge loop with `stability`, `judge_timeout`, `{{ITER_EVIDENCE}}` |
| `generic_consensus.gaviero` | N-reviewer consensus via `reviewers [...]` roster expansion; `consensus_mode` strict/partial_ok/explore |
| `phased_plan.gaviero` | Dynamic phase extraction; per-iteration executor, gate, and judge |
| `codebase_review.gaviero` | Rolling replan/apply loop; `branch_chain stacked`; `until command` termination |
| `update_docs.gaviero` | Parallel doc rewrite; semantic `tier` roles + `profiles/*.gaviero`; `--tiers-file` |
| `security_audit_memory.gaviero` | `memory {}` overrides, additive `read_ns`, `staleness_sources` |

```bash
gaviero-cli --script crates/gaviero-dsl/examples/plan_refinement.gaviero \
    --prompt "Add workspace settings cascade"

gaviero-cli --script crates/gaviero-dsl/examples/update_docs.gaviero \
    --tiers-file crates/gaviero-dsl/examples/profiles/doc-codex.gaviero
```

## Language Reference

### Include

Splice another `.gaviero` file's top-level declarations into the current script — useful for sharing `client {}` profiles, `prompt` templates, and `tier` aliases:

```gaviero
include "lib/clients.gaviero"
include "lib/prompts.gaviero"

agent worker {
    tier   expensive       // resolved via lib/clients.gaviero
    prompt analyse-body    // declared in lib/prompts.gaviero
}

workflow main { steps [worker] }
```

- Paths resolve relative to the directory of the file containing the `include`.
- Cycles are rejected at compile time. Importing the same file twice is idempotent.
- Duplicate top-level names across files are hard errors (both spans shown).
- Includes work only when compiling from a real file path (`--script` / `compile_file`). Inline `compile()` rejects them — there is no anchoring directory.

### Client Block

```gaviero
client opus {
    tier      expensive
    model     "claude:opus"
    privacy   public
    effort    high
    extra { "thinking_budget" "8000" }   // provider pass-through; unknown keys logged
    default                              // used when an agent names no client
}
```

- `effort` — provider-neutral knob: `off` / `auto` / `low` / `medium` / `high` / `xhigh` / `max`.
- `extra { "k" "v" ... }` — provider-specific pairs forwarded verbatim.

### Tier Aliases

Bind a routing label to a client so agents reference an abstract tier:

```gaviero
tier cheap     sonnet
tier expensive opus
```

**Tiers profile file** — put `tier` lines in a separate file and select it at runtime:

```gaviero
// profiles/doc-codex.gaviero — tier lines only
tier inventory       codex-5-5
tier writer_standard codex-5-4
```

```bash
gaviero-cli --script update_docs.gaviero --tiers-file profiles/doc-codex.gaviero
```

The main script must still `include` the client pool. Precedence: CLI `--tiers-file` > included profile > script.

### Workflow Params

Typed parameters on a `workflow {}` block, overridable with `--param NAME=VALUE`:

```gaviero
workflow my-flow {
    param judge { model "claude:sonnet" effort medium privacy public }  // client param
    param roster [                                                       // roster param
        { id "claude" model "claude:opus"    effort max }
        { id "codex"  model "codex:gpt-5.5"  effort high }
    ]
    steps [ loop { reviewers roster template_init t0 template_refine t1 until agent judge } ]
}
```

CLI overrides:

```bash
--param judge=claude:haiku@low
--param roster=claude=claude:opus@max,codex=codex:gpt-5.5@high
```

Bare `param roster` (no `[...]`/`{...}`) infers roster vs client from usage. Required params with no in-script default must be supplied on the CLI.

### Top-level Vars

Compile-time substitution across agent prompts, descriptions, and scope paths. Single-pass — `{{FOO}}` expands once; nested refs in values do not:

```gaviero
vars { PLANS "plans"  VERSION "1" }
```

Override at the CLI with `--var PLANS=output`.

### Model Strings

Canonical form is `provider:model`. Bare names (no `:`) are rejected at compile-dispatch by `gaviero-core`.

| Provider | Examples |
|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus`, `claude:haiku`, `claude:opusplan`, `claude:sonnet[1m]`, `claude:claude-opus-4-7` |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` |
| Cursor | `cursor:claude-4-sonnet` |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:model-name` |
| DeepSeek (API) | `deepseek:deepseek-v4-pro`, `deepseek:deepseek-v4-flash` |

### Scope Block

```gaviero
scope {
    owned        ["src/" "tests/"]
    read_only    ["docs/"]
    impact_scope true    // expand context using the code graph
}
```

- `owned [...]` — files the agent may modify. Entries are glob-style patterns: trailing `/` for a directory prefix, `*`/`?` for single-segment wildcards, `**` across `/`. Two agents overlap only when their patterns could resolve to the same concrete path (`plans/claude-*.md` and `plans/codex-*.md` do **not** overlap).
- `read_only [...]` — extra readable paths (same syntax).
- `impact_scope true` — include the caller/callee graph around owned files.

### Context Block

```gaviero
context {
    callers_of ["src/auth/session.rs"]   // include files that call these targets
    tests_for  ["src/auth/"]             // include related test files
    depth      2                         // graph traversal depth
}
```

### Memory Block

```gaviero
memory {
    read_ns       ["domain-knowledge" "shared"]           // additive with workflow-level read_ns
    write_ns      "current-task"                          // overrides workflow-level write_ns
    importance    0.8                                     // retrieval weight for written memories (0.0–1.0)
    read_query    "architecture decisions and patterns"  // custom semantic search query
    read_limit    15                                      // max memories to retrieve
    write_content #"Summary: {{PROMPT}}"#                 // template for the stored memory text
}
```

`write_content` also supports `{{SUMMARY}}`, `{{FILES}}`, `{{AGENT}}`, and `{{DESCRIPTION}}`, filled in after the agent completes.

### Agent Tools

Request extra tools beyond the runner's default read-only set:

```gaviero
agent checker {
    client sonnet
    tools ["Bash"]    // forwarded verbatim to the backend via --allowedTools
    prompt "Run cargo check and report errors."
}
```

Shell-capable tools bypass write-gate guarantees — use sparingly.

### Verification Block

```gaviero
verify {
    compile      true
    clippy       true
    test         true
    impact_tests true    // only affected tests
}
```

### Workflow Iteration

```gaviero
workflow review_and_fix {
    steps          [reviewer fixer]
    strategy       refine       // single_pass | refine | best_of_N
    max_retries    3
    attempts       2            // for best_of_N
    test_first     true
    escalate_after 2
}
```

### Explicit Loops

```gaviero
loop {
    agents         [fixer verifier]
    max_iterations 5
    iter_start     1
    stability      1
    judge_timeout  120
    strict_judge   true
    branch_chain   stacked    // none (default) | stacked — see examples/codebase_review.gaviero
    until agent    reviewer
}
```

Loop fields: `agents [...]` (body, in order), `iter_start N` (first `{{ITER}}`; `{{PREV_ITER}}` = `ITER − 1`), `stability K` (K consecutive judge PASSes before exit), `judge_timeout N` (per-judge cap in seconds; `0` disables), `strict_judge` (unparseable judge output → hard error vs silent FAIL), `branch_chain stacked` (chain per-iteration git branches so iteration N sees N−1's edits).

Exit conditions:
- `until { compile true test true clippy false impact_tests true }` — verification-based.
- `until agent reviewer` — a judge agent emits `PASS`/`FAIL`, `VERDICT: PASS|FAIL`, or JSON `{"verdict":"pass","reason":"…"}`. The runtime may inject `{{ITER_EVIDENCE}}` (a digest of what changed).
- `until command "cargo test"` — shell command; exit status 0 means the condition is met.

## API

### Entry points

```rust
use gaviero_dsl::{
    compile, compile_with_vars, compile_file,
    workflow_execution_mode, load_tier_overrides,
    CompiledPlan, ExecutionMode,
};

// Inline script (no `include` statements)
let plan = compile(source, filename, Some("workflow_name"), Some("runtime prompt"))?;

// Inline script + CLI overrides (--var / --tiers-file / --param)
let plan = compile_with_vars(
    source, filename, Some("workflow_name"), Some("runtime prompt"),
    &override_vars,    // &[(String, String)] — --var KEY=VALUE
    &override_tiers,   // &[(String, String)] — --tiers-file bindings
    &override_params,  // &[(String, String)] — --param NAME=VALUE
)?;

// From disk — resolves `include "…"` transitively (use this for real files)
let plan = compile_file(
    std::path::Path::new("examples/plan_refinement.gaviero"),
    Some("feature-plan-refinement"),  // workflow name
    Some("runtime prompt text"),      // {{PROMPT}} substitution
    &[],                              // override_vars
    &[],                              // override_tiers
    &[],                              // override_params
)?;

// Peek a script's execution mode (repo vs document) without full compilation
let mode: ExecutionMode = workflow_execution_mode(path, Some("my-workflow"))?;

// Load a tier-overrides profile (backs --tiers-file)
let tiers = load_tier_overrides(std::path::Path::new("profiles/doc-codex.gaviero"))?;
```

All compile entry points return `Result<CompiledPlan, miette::Report>` with colorful source diagnostics on failure.

- **Var precedence:** agent-level `vars {}` > CLI `--var` (`override_vars`) > script-level `vars {}`.
- **Tier precedence:** CLI `--tiers-file` (`override_tiers`) > script/includes `tier` lines.
- **Params:** roster `id=provider:model[@effort],...`; client `provider:model[@effort]`. Required params without an in-script default fail compilation if absent.
- `compile` / `compile_with_vars` **reject** `include` statements with a diagnostic pointing to `compile_file`.

### Return type

```rust
use gaviero_dsl::CompiledPlan;   // re-exported from gaviero_core::swarm::plan

pub struct CompiledPlan {
    pub nodes: HashMap<String, PlanNode>,
    pub edges: Vec<(String, String)>,
    pub root:  Vec<String>,
    // ... metadata
}
```

`ExecutionMode` (repo vs document) and the deprecated `CompiledScript` alias are also re-exported for backward compatibility.

## Configuration

The DSL itself has no runtime config — behaviour is fully declared in the script and adjusted at compile time through `--var`, `--tiers-file`, and `--param` overrides (see the CLI flags in [`gaviero-cli`](../gaviero-cli/README.md)). Provider resolution, memory namespaces, and verification execution are configured on the `gaviero-core` side via `.gaviero/settings.json`.

## Design

**What this crate does:** tokenization (`logos`), parsing (`chumsky` combinators), semantic validation (scope overlaps, cycles), include resolution with cycle detection, tier-override and workflow-param loading, and compilation to a `CompiledPlan` DAG.

**What it does NOT do:** runtime execution, provider model resolution, verification-gate execution, or git operations — all of that is `gaviero-core`.

## Dependencies

- `gaviero-core` — `CompiledPlan`, `path_pattern::paths_overlap`, shared types
- `logos` — lexer generator
- `chumsky` — parser combinators
- `miette` + `thiserror` — diagnostic error display

## See Also

- [`crates/gaviero-core/README.md`](../gaviero-core/README.md) — execution runtime
- [`crates/gaviero-cli/README.md`](../gaviero-cli/README.md) — `--script`, `--var`, `--param`, `--tiers-file`
- [ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline diagram

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
