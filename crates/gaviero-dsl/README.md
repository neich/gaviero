# gaviero-dsl

Compiler for declarative `.gaviero` workflow scripts. Define multi-agent tasks with scopes, dependencies, verification, and iteration strategies. The compiler transforms DSL source into execution DAGs consumed by the swarm runtime.

## Installation & Build

```bash
cargo build -p gaviero-dsl
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Core Concepts

**Client** — Define model, tier, and privacy defaults for groups of agents:
```gaviero
client reasoning { tier expensive model "sonnet" }
```

**Agent** — A unit of work with scope, prompt, dependencies, and optional blocks:
```gaviero
agent design { description "..." client reasoning scope {...} prompt "..." }
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

### Model Strings

Provider-neutral: resolved at runtime by gaviero-core.

- **Claude** — `sonnet`, `opus`, `haiku` (shorthand) or `claude:sonnet`, `claude-code:haiku` (explicit)
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

- `owned [...]` — files the agent may modify
- `read_only [...]` — extra readable paths
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
    namespace "domain-knowledge"
    write_namespace "current-task"
    trust "high"
    limit 5
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
- `until agent reviewer` — human judgment (agent decision)
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

### Main entry point

```rust
use gaviero_dsl::compile;

let plan = compile(source, filename, workflow_name, runtime_prompt)?;
// plan is a CompiledPlan DAG ready for swarm execution
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
