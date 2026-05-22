# gaviero-dsl

Compiler for declarative `.gaviero` workflow scripts. Define multi-agent tasks with scopes, dependencies, verification, and iteration strategies. The compiler transforms DSL source into execution DAGs consumed by the swarm runtime.

## Installation & Build

```bash
cargo build -p gaviero-dsl
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Core Concepts

**Client** — Define model, tier, effort, and optional provider-specific extras:
```gaviero
client reasoning { tier expensive model "claude:opus"   effort high default }
client fast      { tier cheap     model "claude:sonnet" effort low }
```

**Tier alias** — Name a routing label and bind it to a client:
```gaviero
tier cheap     fast
tier expensive reasoning
```

**Vars** — Script-level key/value substitutions applied across all agents:
```gaviero
vars {
    PLANS   "plans"
    VERSION "1"
}
```

**Prompt** — Named, reusable prompt templates with `{{VAR}}` substitution:
```gaviero
prompt review-body #"
    Review {{PLANS}}/{{MODEL}}-draft.md and list all issues.
"#
```

**Agent** — A unit of work with scope, prompt, dependencies, and optional blocks:
```gaviero
agent design { description "..." client reasoning scope {...} prompt review-body }
```

**Workflow** — Orchestrates agents with execution strategy, verification, and loop rules:
```gaviero
workflow review_and_fix { steps [design fixer] verify {...} }
```

## Examples

### Basic workflow with two agents

Create a file `refactor.gaviero`:

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
    scope {
        read_only ["src/" "tests/" "docs/"]
    }
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
    verify {
        compile      true
        clippy       true
        impact_tests true
    }
}
```

### Example scripts in `examples/`

Six workflows ship with the crate. They use `include "clients.gaviero"` and must be
compiled via `compile_file` or `gaviero-cli --script` (inline `compile()` rejects
includes).

| File | Demonstrates |
|------|----------------|
| `clients.gaviero` | Shared profiles: `claude:opus`, `claude:sonnet`, `codex:gpt-5.5`, `codex:gpt-5.4` |
| `plan_refinement.gaviero` | Dual-model plan refinement; judge loop with `stability`, `judge_timeout`, `{{ITER_EVIDENCE}}` |
| `generic_consensus.gaviero` | N-reviewer generic consensus via `reviewers [...]` roster expansion; `consensus_mode` strict/partial_ok/explore |
| `phased_plan.gaviero` | Dynamic phase extraction; per-iteration executor, gate, and judge |
| `codebase_review.gaviero` | Rolling replan/apply loop; `branch_chain stacked`; `until command` termination |
| `update_docs.gaviero` | Parallel doc rewrite; semantic `tier` roles + `profiles/*.gaviero`; `--tiers-file` |
| `security_audit_memory.gaviero` | `memory {}` overrides, additive `read_ns`, `staleness_sources` |

```bash
gaviero-cli --script crates/gaviero-dsl/examples/plan_refinement.gaviero \
    --prompt "Add workspace settings cascade"

gaviero-cli --script crates/gaviero-dsl/examples/update_docs.gaviero
gaviero-cli --script crates/gaviero-dsl/examples/update_docs.gaviero \
    --tiers-file crates/gaviero-dsl/examples/profiles/doc-codex.gaviero
```

## Language Reference

### Include

Splice another `.gaviero` file's top-level declarations into the current
script. Useful for sharing `client {}` profiles, `prompt` templates, and
`tier` aliases across multiple workflow scripts:

```gaviero
include "lib/clients.gaviero"
include "lib/prompts.gaviero"

agent worker {
    tier expensive          // resolved via lib/clients.gaviero
    prompt analyse-body     // declared in lib/prompts.gaviero
}

workflow main { steps [worker] }
```

- Paths are resolved relative to the directory of the file containing the
  `include`. `"lib/x.gaviero"` from `/proj/main.gaviero` reads `/proj/lib/x.gaviero`.
- Cycles are rejected at compile time. Importing the same file from two
  different paths is idempotent — its items are merged exactly once.
- Duplicate top-level names across files are hard errors with both spans
  shown. Either rename or prefix names per library to avoid collisions.
- Includes work only when compiling from a real file path:
  `gaviero-cli --script main.gaviero` or `gaviero_dsl::compile_file(path, …)`.
  Inline `compile()` (used by tests / fenced markdown blocks) rejects them
  because there's no anchoring directory to resolve against.

### Client Block

Declares a model, tier, effort level, and optional provider-specific extras:

```gaviero
client opus {
    tier      expensive
    model     "claude:opus"
    privacy   public
    effort    high
    extra {
        "thinking_budget" "8000"   // provider pass-through; unknown keys logged
    }
    default    // used when no client is specified on an agent
}
```

- `effort` — provider-neutral knob: `off` / `auto` / `low` / `medium` / `high` / `xhigh` / `max`
- `extra { "k" "v" ... }` — provider-specific key/value pairs forwarded verbatim

### Tier Aliases

Bind a routing label to a client so agents reference an abstract tier rather than a specific model:

```gaviero
tier cheap     sonnet
tier expensive opus
```

Agents using `tier expensive` are re-routed by changing one line.

**Tiers profile file** — put bindings in a separate file and select it at runtime:

```gaviero
// profiles/doc-codex.gaviero — tier lines only
tier inventory       codex-5-5
tier writer_standard codex-5-4
```

```bash
gaviero-cli --script update_docs.gaviero \
    --tiers-file profiles/doc-codex.gaviero
```

The main script must still `include` the client pool (`clients.gaviero`). `--tiers-file`
overrides `tier` lines from the script and its includes; precedence is CLI profile >
included profile > script.

### Workflow Params

Typed parameters on a `workflow { ... }` block, overridable from the CLI with
`--param NAME=VALUE`:

```gaviero
workflow my-flow {
    // Client param — bind agents with `client judge`
    param judge {
        model "claude:sonnet"
        effort medium
        privacy public
    }

    // Roster param — use in `loop { reviewers roster ... }`
    param roster [
        { id "claude" model "claude:opus" effort max }
        { id "codex"  model "codex:gpt-5.5" effort high }
    ]

    steps [ loop { reviewers roster template_init t0 template_refine t1 until agent judge } ]
}
```

CLI overrides:

```bash
--param judge=claude:haiku@low
--param roster=claude=claude:opus@max,codex=codex:gpt-5.5@high
```

Bare `param roster` (no `[...]` or `{...}`) infers roster vs client from whether the
name appears in `reviewers` or `client` on an agent. Required params without an
in-script default must be supplied on the CLI.

### Top-level Vars

Script-level substitution applied to agent prompts, descriptions, and scope paths before compilation:

```gaviero
vars {
    PLANS   "plans"
    VERSION "1"
}
```

Override at the CLI with `--var PLANS=output`.

### Named Prompts

Reusable prompt templates referenced by name in agents:

```gaviero
prompt review-body #"
    Review {{PLANS}}/{{MODEL}}-draft.md for correctness.
"#

agent reviewer {
    client opus
    prompt review-body    // reference by name
}
```

### Model Strings

Canonical form is `provider:model`. Bare names (no `:`) are rejected at
compile-dispatch time by `gaviero-core`.

- **Claude** — `claude:sonnet`, `claude:opus`, `claude:haiku`,
  `claude:opusplan`, `claude:sonnet[1m]`, or any versioned form like
  `claude:claude-opus-4-7`
- **Codex** — `codex:<model>` (e.g., `codex:gpt-5.5`, `codex:gpt-5.4`)
- **Cursor** — `cursor:<model>` (e.g., `cursor:claude-4-sonnet`)
- **Ollama/local** — `ollama:qwen2.5-coder:7b` or `local:model-name`

### Scope Block

Declare file ownership and read boundaries:

```gaviero
scope {
    owned        ["src/" "tests/"]
    read_only    ["docs/"]
    impact_scope true    // expand context using code graph
}
```

- `owned [...]` — files the agent may modify. Entries are glob-style
  patterns: trailing `/` for a directory prefix, `*`/`?` for single-segment
  wildcards, `**` to match across `/`. Two agents overlap only when their
  patterns could resolve to the same concrete path (e.g.
  `plans/claude-*.md` and `plans/codex-*.md` do **not** overlap).
- `read_only [...]` — extra readable paths (same pattern syntax)
- `impact_scope true` — include caller/callee graph around owned files

### Context Block

Drive context selection with code graph analysis:

```gaviero
context {
    callers_of  ["src/auth/session.rs"]
    tests_for   ["src/auth/"]
    depth       2
}
```

- `callers_of [...]` — include files that call specified targets
- `tests_for [...]` — include test files related to targets
- `depth <n>` — graph traversal depth

### Memory Block

Control semantic memory reads and writes:

```gaviero
memory {
    read_ns       ["domain-knowledge" "shared"]   // additive with workflow-level read_ns
    write_ns      "current-task"                   // overrides workflow-level write_ns
    importance    0.8                              // retrieval weight for written memories (0.0–1.0)
    read_query    "architecture decisions and patterns"  // custom semantic search query
    read_limit    15                               // max memories to retrieve
    write_content #"Summary: {{PROMPT}}"#          // template for the stored memory text
}
```

`write_content` also supports `{{SUMMARY}}`, `{{FILES}}`, `{{AGENT}}`, and
`{{DESCRIPTION}}` (filled in by the runtime after the agent completes).

### Agent tools

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

Specify post-execution checks:

```gaviero
verify {
    compile      true
    clippy       true
    test         true
    impact_tests true  // only affected tests
}
```

### Workflow Iteration

Control execution strategy and retries:

```gaviero
workflow review_and_fix {
    steps        [reviewer fixer]
    strategy     refine         // single_pass | refine | best_of_N
    max_retries  3
    attempts     2              // for best_of_N
    test_first   true
    escalate_after 2
}
```

### Explicit Loops

Repeat a sequence of agents with exit conditions:

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

Loop fields:
- `agents [...]` — body agents executed each iteration (in order)
- `iter_start N` — first value of `{{ITER}}` (default `1`); `{{PREV_ITER}}` is `ITER - 1`
- `stability K` — require K consecutive judge PASSes before exit (`until agent` only)
- `judge_timeout N` — hard cap per judge invocation in seconds (`0` disables)
- `strict_judge true|false` — unparseable judge output: hard error vs silent FAIL
- `branch_chain stacked` — chain per-iteration git branches so iteration N sees
  iteration N−1's source edits (required for source-mutating rolling loops)

Exit conditions:
- `until { compile true test true clippy false impact_tests true }` — verification-based exit
- `until agent reviewer` — run a judge agent on-demand; emit `PASS` / `FAIL`,
  `VERDICT: PASS|FAIL`, or JSON like `{"verdict":"pass","reason":"…"}`. The runtime
  may inject `{{ITER_EVIDENCE}}` into the judge prompt (digest of what changed).
- `until command "cargo test"` — shell command; exit status 0 means condition met

## Running Workflows

### From the TUI editor

```bash
/run path/to/workflow.gaviero
/run path/to/workflow.gaviero "runtime prompt text"
```

The runtime prompt is substituted for `{{PROMPT}}` placeholders in agent prompts.

### From the CLI

```bash
gaviero-cli --script path/to/workflow.gaviero
gaviero-cli --script workflows/refactor.gaviero
```

### Generate from coordinated planning

Let the coordinator generate a plan automatically:

```bash
gaviero-cli --coordinated --task "refactor the auth module"
```

The output is a `.gaviero` file you can inspect, edit, and then execute.

## API Overview

### Entry points

```rust
use gaviero_dsl::{compile, compile_file, compile_with_vars, load_tier_overrides};

// Inline script (no `include` statements)
let plan = compile(source, filename, workflow_name, runtime_prompt)?;

// From disk — resolves `include "…"` transitively
let plan = compile_file(
    std::path::Path::new("examples/plan_refinement.gaviero"),
    Some("feature-plan-refinement"),
    Some("runtime prompt text"),
    &[],
    &[],
)?;

// CLI-level var overrides (beat script-level vars {}; agent-level vars still win)
let overrides = vec![("PLANS".to_string(), "output".to_string())];
let plan = compile_with_vars(source, filename, workflow_name, runtime_prompt, &overrides, &[])?;

// Load a tier-overrides profile (backs --tiers-file)
let tiers = load_tier_overrides(std::path::Path::new("profiles/doc-codex.gaviero"))?;
let plan = compile_with_vars(source, filename, workflow_name, runtime_prompt, &[], &tiers)?;
```

### Return type

```rust
use gaviero_core::swarm::plan::CompiledPlan;

pub struct CompiledPlan {
    pub nodes: HashMap<String, PlanNode>,
    pub edges: Vec<(String, String)>,
    pub root: Vec<String>,
    // ... metadata
}
```

## Design

**What this crate does:**
- Tokenization via `logos` lexer
- Parsing via `chumsky` parser combinators
- Semantic validation (scope overlaps, cycles)
- Include resolution with cycle detection (`resolver.rs`)
- Tier-override loading for `--tiers-file` profiles (`tiers.rs`)
- Compilation to `CompiledPlan` DAG

**What it does NOT do:**
- Runtime execution (gaviero-core)
- Provider model resolution (gaviero-core)
- Verification gate execution (gaviero-core)
- Git operations (gaviero-core)

## Dependencies

- `gaviero-core` — types (`CompiledPlan`, `WorkUnit`)
- `logos` — lexer generator
- `chumsky` — parser combinator library
- `miette` — diagnostic error display
- `thiserror` — error derivation

## See Also

- [ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline diagram
- [crates/gaviero-core/README.md](../gaviero-core/README.md) — execution runtime
