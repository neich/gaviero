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
client reasoning { tier expensive model "claude-opus-4-7" effort high default }
client fast      { tier cheap     model "claude-sonnet-4-6" effort low }
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
    model "sonnet"
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

## Language Reference

### Client Block

Declares a model, tier, effort level, and optional provider-specific extras:

```gaviero
client opus {
    tier      expensive
    model     "claude-opus-4-7"
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

Provider-neutral: resolved at runtime by gaviero-core.

- **Claude** — `sonnet`, `opus`, `haiku` (shorthand) or `claude:sonnet`, `claude-code:haiku` (explicit)
- **Codex** — `codex:<model>` (e.g., `codex:gpt-5.4`)
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

Repeat a workflow or agent with exit conditions:

```gaviero
loop {
    max_iterations 5
    steps [fixer verifier]
    until verification {
        compile  true
        test     true
    }
}
```

Exit conditions:
- `until verification { ... }` — verification-based exit
- `until agent reviewer` — run a judge agent on-demand; emit `PASS` / `FAIL`,
  `VERDICT: PASS|FAIL`, or JSON like `{"pass": true}`
- `until command "cargo test"` — shell command exit status

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
use gaviero_dsl::{compile, compile_with_vars};

// Basic compilation
let plan = compile(source, filename, workflow_name, runtime_prompt)?;

// With CLI-level var overrides (beat script-level vars {}, lose to agent-level vars {})
let overrides = vec![("PLANS".to_string(), "output".to_string())];
let plan = compile_with_vars(source, filename, workflow_name, runtime_prompt, &overrides)?;
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
