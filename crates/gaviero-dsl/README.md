# gaviero-dsl

A declarative language for composing multi-agent AI workflows. Write `.gaviero` files that describe `client` backends, `agent` tasks, and `workflow` orchestration. The compiler produces a `CompiledPlan` that Gaviero's swarm engine executes.

`.gaviero` files are also the output of coordinated planning. When you run `/cswarm <task>` in the TUI or `gaviero-cli --coordinated --task "..."`, Opus decomposes the task into a `.gaviero` file saved to `tmp/` for your review before any agents run.

---

## Quick example

```gaviero
client sonnet { tier cheap  model "claude-sonnet-4-6" }
client opus   { tier expensive model "claude-opus-4-6" }

agent reviewer {
    description "Review the PR and identify issues"
    client opus
    scope {
        read_only ["src/" "tests/"]
    }
    prompt #"
        Review the code changes and list all bugs, missing tests, and style issues.
        Output a numbered list only.
    "#
}

agent fixer {
    description "Fix all issues found by the reviewer"
    client sonnet
    depends_on [reviewer]
    scope {
        owned     ["src/" "tests/"]
        read_only ["docs/"]
        impact_scope true     // auto-expand read_only with blast-radius files
    }
    context {
        callers_of ["src/auth/session.rs"]   // include callers of these files
        tests_for  ["src/auth/"]             // include associated test files
        depth      2
    }
    prompt "Fix every issue in the reviewer's list. Do not change anything else."
    max_retries 3
}

workflow review_and_fix {
    steps [reviewer fixer]
    verify {
        compile      true
        clippy       true
        impact_tests true    // run only tests affected by modified files
    }
}
```

Run with `gaviero-cli --script review_and_fix.gaviero` or `/run review_and_fix.gaviero` in the TUI.

---

## Complete workflow examples

### 1. Bug fix with TDD — `bugfix_with_tests.gaviero`

The canonical single-agent workflow. The agent generates failing tests first, then fixes the code until they pass.

```gaviero
client haiku { tier cheap model "claude-haiku-4-5-20251001" }

agent fix {
    description "Fix the authentication bug: {{PROMPT}}"
    client haiku
    scope {
        owned    ["src/auth/" "tests/auth/"]
        read_only ["src/types.rs" "docs/auth.md"]
    }
    prompt #"
        There is a bug in the authentication module: {{PROMPT}}

        Step 1 — Write a test that reproduces the bug. The test must fail
        against the current code and must compile.

        Step 2 — Fix the code so the test passes without breaking any
        existing tests.
    "#
    max_retries 5
}

workflow bugfix {
    steps [fix]
    test_first  true
    strategy    refine
    max_retries 5
    verify {
        compile true
        clippy  true
        test    true
    }
}
```

Run:
```bash
gaviero-cli --script bugfix_with_tests.gaviero \
  --task "tokens are not invalidated on logout"
```

The `{{PROMPT}}` placeholder is replaced with the `--task` value at compile time.

---

### 2. New feature with test-first TDD — `feature_tdd.gaviero`

Opus writes the specification as failing tests; Haiku implements until they pass; a final Sonnet agent validates edge cases.

```gaviero
client haiku   { tier cheap     model "claude-haiku-4-5-20251001" }
client sonnet  { tier cheap     model "claude-sonnet-4-6" }
client opus    { tier expensive model "claude-opus-4-6" }

agent spec {
    description "Write failing tests that specify: {{PROMPT}}"
    client opus
    scope {
        owned    ["tests/billing/"]
        read_only ["src/billing/" "src/types.rs"]
    }
    prompt #"
        Write comprehensive tests for the following feature: {{PROMPT}}
        Tests MUST fail against the current code and MUST compile.
        Do NOT modify any source files.
    "#
    max_retries 2
}

agent implement {
    description "Implement until tests pass"
    client haiku
    depends_on  [spec]
    scope {
        owned    ["src/billing/"]
        read_only ["tests/billing/" "src/types.rs" "docs/"]
        impact_scope true
    }
    context {
        callers_of ["src/billing/"]
        depth      2
    }
    prompt #"
        The tests in tests/billing/ describe the desired behaviour.
        Implement the code in src/billing/ to make all tests pass.
        Do not modify the test files.
    "#
    max_retries 8
}

agent harden {
    description "Add error handling and edge-case coverage"
    client sonnet
    depends_on  [implement]
    scope {
        owned    ["src/billing/" "tests/billing/"]
        read_only ["src/types.rs"]
    }
    prompt #"
        Review the implementation and tests for {{PROMPT}}.
        Add: missing error handling, missing edge cases, documentation.
        All existing tests must still pass.
    "#
    max_retries 3
}

workflow tdd_feature {
    steps        [spec implement harden]
    strategy     refine
    escalate_after 3

    verify {
        compile      true
        clippy       true
        test         true
        impact_tests true
    }

    memory {
        read_ns  ["architecture" "coding-patterns"]
        write_ns "feature-billing"
    }
}
```

Run:
```bash
gaviero-cli --script feature_tdd.gaviero \
  --task "subscription billing with proration support"
```

---

### 3. Best-of-3 sampling for risky refactoring — `refactor_safe.gaviero`

When the correct refactoring approach is ambiguous, generate three independent attempts and keep the one that passes all gates.

```gaviero
client haiku { tier cheap model "claude-haiku-4-5-20251001" }

agent refactor {
    description "Refactor {{PROMPT}}"
    client haiku
    scope {
        owned    ["src/core/"]
        read_only ["tests/core/" "src/types.rs"]
        impact_scope true
    }
    prompt #"
        Refactor the following: {{PROMPT}}

        Constraints:
        - All existing tests must pass unchanged
        - No change to public API signatures
        - Apply Rust idiomatic patterns (iterators, ? operator, etc.)
    "#
    max_retries 4
}

workflow refactor_safe {
    steps          [refactor]
    strategy       best_of_3
    max_retries    4
    escalate_after 2

    verify {
        compile      true
        clippy       true
        test         true
        impact_tests true
    }
}
```

Run:
```bash
gaviero-cli --script refactor_safe.gaviero \
  --task "replace manual error mapping in the parser module with thiserror derive"
```

The engine runs three independent agents. If any attempt passes all verification gates, it is returned immediately. Otherwise the attempt with the most modified files is returned.

---

### 4. Multi-crate refactor with dependencies — `multi_crate.gaviero`

Explicit multi-agent workflow. Each agent owns a disjoint scope; an integration agent runs after both finish.

```gaviero
client haiku  { tier cheap     model "claude-haiku-4-5-20251001" }
client sonnet { tier cheap     model "claude-sonnet-4-6" }

agent core-types {
    description "Rename ModelTier variants in gaviero-core"
    client haiku
    scope { owned ["crates/gaviero-core/src/"] }
    prompt #"
        Rename the ModelTier enum variants: Execution → Cheap, Coordinator → Expensive.
        Update every match arm, struct field, and test that references the old names.
    "#
    max_retries 3
}

agent dsl-update {
    description "Update DSL compiler to emit new ModelTier values"
    client haiku
    scope {
        owned     ["crates/gaviero-dsl/src/"]
        read_only ["crates/gaviero-core/src/types.rs"]
    }
    prompt #"
        Update the DSL compiler to map tier literals to the renamed ModelTier
        variants: Cheap and Expensive.
    "#
    max_retries 3
}

agent integration-check {
    description "Fix any compilation errors across the whole workspace"
    client sonnet
    depends_on  [core-types dsl-update]
    scope {
        owned    ["crates/"]
        read_only ["Cargo.toml" "Cargo.lock"]
    }
    prompt #"
        The ModelTier enum was renamed across two crates.
        Fix any remaining compilation errors. Do not change logic — only fix name references.
    "#
    max_retries 5
}

workflow multi_crate_rename {
    steps        [core-types dsl-update integration-check]
    max_parallel 2
    strategy     refine

    verify { compile true clippy true test true }
}
```

Run:
```bash
gaviero-cli --script multi_crate.gaviero --max-parallel 2
```

---

### 5. Security audit — `security_audit.gaviero`

Read-only audit that writes its findings to a markdown report and to memory.

```gaviero
client opus { tier expensive model "claude-opus-4-6" }

agent audit {
    description "Security audit of the authentication layer"
    client opus
    scope {
        owned    ["docs/security-audit.md"]
        read_only ["src/auth/" "src/middleware/" "tests/"]
    }
    context {
        callers_of ["src/auth/session.rs" "src/middleware/auth.rs"]
        depth      3
    }
    prompt #"
        Perform a security audit of the authentication implementation.
        Review for: injection vulnerabilities, authentication bypass, session issues,
        insecure defaults, missing validation, timing attacks.
        Write findings to docs/security-audit.md.
    "#
    max_retries 1

    memory {
        write_ns          "security-audits"
        importance        0.9
        staleness_sources ["src/auth/" "src/middleware/"]
    }
}

workflow security_audit {
    steps    [audit]
    strategy single_pass
}
```

Run:
```bash
gaviero-cli --script security_audit.gaviero --model opus
```

---

### 6. Iterative fix loop — `loop_fix.gaviero`

An explicit loop reruns a block of agents until a condition is met.

```gaviero
client sonnet { tier cheap model "claude-sonnet-4-6" }

agent implement {
    description "Implement feature: {{PROMPT}}"
    client sonnet
    scope { owned ["src/"] read_only ["tests/"] }
    prompt #"
        Implement the following feature: {{PROMPT}}
        Make all existing tests pass and add new tests if needed.
    "#
    max_retries 3

    memory {
        read_ns     ["prior-attempts"]
        read_query  "previous implementation failures for {{PROMPT}}"
        read_limit  10
        write_ns    "prior-attempts"
        importance  0.8
    }
}

agent verify {
    description "Verify the implementation"
    client sonnet
    depends_on [implement]
    scope { read_only ["src/" "tests/"] }
    prompt "Review the implementation for correctness and edge cases."
    max_retries 1
}

workflow iterative_fix {
    steps [
        loop {
            agents [implement verify]
            max_iterations 5
            until { compile true test true }
        }
    ]
    max_parallel 1
}
```

Run:
```bash
gaviero-cli --script loop_fix.gaviero \
  --task "add pagination to the user list endpoint"
```

---

### 7. Local-only (no API) workflow — `local_refactor.gaviero`

Uses Ollama for privacy-sensitive code. No data leaves the machine.

```gaviero
client local {
    tier     cheap
    model    "qwen2.5-coder:14b"
    privacy  local_only
}

agent refactor {
    description "Refactor {{PROMPT}} using local model"
    client local
    scope { owned ["src/internal/"] }
    prompt "Refactor: {{PROMPT}}. Keep the public API unchanged."
    max_retries 3
}

workflow local_refactor {
    steps    [refactor]
    strategy refine
    verify   { compile true }
}
```

Requires a running Ollama instance (`ollama serve`).

---

## Declarations

### `client` — LLM backend configuration

```gaviero
client <name> {
    tier    cheap | expensive          // model tier (default: cheap)
    model   "<model-id>"               // override specific model
    privacy public | local_only        // public: any API; local_only: Ollama only
}
```

**Tier values:**

| Value | Maps to | Typical model |
|---|---|---|
| `cheap` | `ModelTier::Cheap` | claude-haiku-4-5 |
| `expensive` | `ModelTier::Expensive` | claude-sonnet-4-6 |
| `coordinator`, `reasoning` | `Expensive` (deprecated aliases) | |
| `execution`, `mechanical` | `Cheap` (deprecated aliases) | |

---

### `agent` — Work unit

```gaviero
agent <name> {
    description "<task description>"         // shown in UI; {{PROMPT}} substitution
    client      <client-name>
    scope {
        owned        ["path/" "file.rs"]     // agent can read and write these
        read_only    ["docs/" "other.md"]    // agent can read but not write
        impact_scope true                    // auto-expand read_only with blast-radius files
    }
    context {                                // explicit code-graph queries (optional)
        callers_of ["src/auth/session.rs"]  // include callers of these files in context
        tests_for  ["src/auth/"]            // include test files associated with these paths
        depth      2                         // BFS traversal depth (default: 2)
    }
    depends_on  [agent-a agent-b]            // wait for these to complete first
    prompt #"
        Multi-line raw string instructions.
        {{PROMPT}} is replaced with the --task / runtime_prompt value.
    "#
    max_retries 3                            // inner validation-feedback cycles (default: 1)
    memory {
        read_ns           ["ns-1" "ns-2"]    // namespaces to search at run time
        write_ns          "my-ns"            // namespace for storing results
        importance        0.8                // memory importance weight (0.0–1.0)
        staleness_sources ["src/"]           // invalidate memory if these paths change
        read_query        "custom search query"   // override auto-query (default: description)
        read_limit        10                 // max search results (default: 5)
        write_content #"                     // custom write template (default: auto-summary)
            Agent: {{AGENT}}
            Summary: {{SUMMARY}}
            Files: {{FILES}}
        "#
    }
}
```

If `scope` is omitted the agent gets `owned = ["."]` (full workspace). `impact_scope` and `context {}` are optional — omit them for simple agents.

---

### `workflow` — Orchestration

```gaviero
workflow <name> {
    steps [
        agent-a
        agent-b
        loop {                                         // explicit loop block
            agents [agent-c agent-d]                   // agents to repeat
            max_iterations 5                           // hard upper bound
            until { compile true test true }            // exit condition
        }
        agent-e
    ]
    max_parallel 2                             // max concurrent agents (default: 1)

    // Iteration strategy
    strategy     single_pass | refine | best_of_N    // default: refine
    max_retries  5                             // inner retries per attempt (default: 5)
    attempts     3                             // outer attempts for BestOfN (default: 1)
    test_first   true                          // generate failing tests before editing
    escalate_after 2                           // switch to expensive model after N failures

    // Post-edit verification
    verify {
        compile      true      // run cargo check / equivalent
        clippy       true      // run cargo clippy
        test         false     // run full test suite
        impact_tests false     // run only tests affected by modified files
    }

    // Workflow-level memory defaults (merged with agent-level settings)
    memory {
        read_ns  ["shared" "policies"]
        write_ns "default-ns"
    }
}
```

---

## `scope` fields

| Field | Type | Default | Description |
|---|---|---|---|
| `owned` | string list | `["."]` | Paths the agent may read and write |
| `read_only` | string list | `[]` | Paths the agent may only read |
| `impact_scope` | bool | `false` | When `true`, automatically expand `read_only` with blast-radius files computed from the code knowledge graph |

---

## `context` block

The `context {}` block inside an agent declaration queries the code knowledge graph to inject additional files into the agent's prompt context beyond what the scope declares.

| Field | Type | Default | Description |
|---|---|---|---|
| `callers_of` | string list | `[]` | Files whose callers (up to `depth` hops) are included in context |
| `tests_for` | string list | `[]` | Paths whose associated test files are included in context |
| `depth` | integer | `2` | BFS traversal depth for graph queries |

---

## `verify` fields

| Field | Type | Default | Description |
|---|---|---|---|
| `compile` | bool | `false` | Run `cargo check` (or equivalent) after each agent write |
| `clippy` | bool | `false` | Run `cargo clippy` |
| `test` | bool | `false` | Run the full test suite |
| `impact_tests` | bool | `false` | Run only tests whose source files were modified (faster than `test true`) |

`impact_tests` and `test` are independent; you can enable both, but typically only one is needed.

---

## Strategies

| Strategy | Behaviour |
|---|---|
| `single_pass` | One attempt, no inner retry |
| `refine` | One attempt with up to `max_retries` validation-feedback cycles (default) |
| `best_of_3` | 3 independent attempts; returns the first to pass, or the one with the most file changes |

`best_of_N` where N is any integer: `best_of_2`, `best_of_5`, etc.

**When to use which:**

| Scenario | Recommended strategy |
|---|---|
| Well-scoped bug fix | `refine` — fast, iterates toward correctness |
| Ambiguous refactoring | `best_of_3` — explore multiple approaches |
| Read-only audit or analysis | `single_pass` — no iteration needed |
| New feature with tests | `refine` + `test_first true` — TDD loop |
| Critical code, high confidence needed | `best_of_5` + `test true` |

---

## Loops

A `loop { }` block inside `steps [...]` repeats a sequence of agents until an exit condition is met or `max_iterations` is reached.

```gaviero
workflow fix_cycle {
    steps [
        analyze
        loop {
            agents [implement test_runner]
            max_iterations 5
            until { compile true test true }
        }
        deploy
    ]
}
```

### Exit conditions

| Form | Behaviour |
|---|---|
| `until { compile true test true }` | Verify block — exits when the specified checks pass |
| `until agent quality_gate` | Judge agent — runs a named agent that decides pass/fail |
| `until command "cargo test --quiet"` | Shell command — exits when the command returns exit code 0 |

`max_iterations` is always required as a safety bound.

---

## Explicit memory control

| Field | Type | Default | Description |
|---|---|---|---|
| `read_query` | string | agent description | Custom semantic search query. Supports `{{PROMPT}}` |
| `read_limit` | integer | 5 | Maximum number of search results |
| `write_content` | string | auto-generated summary | Template for written content |

### Write content template variables

| Variable | Replaced with |
|---|---|
| `{{SUMMARY}}` | The agent's full text output |
| `{{FILES}}` | Comma-separated list of modified files |
| `{{AGENT}}` | The agent's name/ID |
| `{{DESCRIPTION}}` | The agent's description field |

### Namespace merge rules

| Field | Rule |
|---|---|
| `read_ns` | Additive: workflow list + agent list, deduplicated |
| `write_ns` | Agent value overrides workflow value |
| `importance` | Agent-only |
| `staleness_sources` | Agent-only |
| `read_query`, `read_limit`, `write_content` | Agent-only; `{{PROMPT}}` substituted at compile time |

---

## Execution without a workflow

- **No workflow declaration** — all agents run in declaration order.
- **Exactly one workflow** — used automatically.
- **Multiple workflows** — pass `--workflow <name>` (CLI) to select one.

---

## `{{PROMPT}}` substitution

Any `description` or `prompt` field may contain `{{PROMPT}}`. Replaced at compile time by the `runtime_prompt` argument — the value of `--task` on the CLI, or the message sent to `/run` in the TUI.

```bash
# Same script, different task each time
gaviero-cli --script bugfix.gaviero --task "null pointer in UserService.getById()"
gaviero-cli --script bugfix.gaviero --task "race condition in connection pool shutdown"
```

---

## Dependency edges

`depends_on` creates a directed edge in the execution DAG. Agents with no dependencies run in parallel (subject to `max_parallel`). Cycles are detected at compile time. An agent is skipped if any dependency failed (`AllSuccess` trigger rule).

---

## DSL syntax reference

| Construct | Syntax |
|---|---|
| String | `"value"` |
| Raw string | `#"multi line\nno escapes"#` |
| String list | `["a" "b" "c"]` — no commas |
| Ident list | `[agent-a agent-b]` — no commas |
| Integer | `42` |
| Float | `0.8` |
| Boolean | `true` / `false` |
| Comment | `// line comment` |
| Loop block | `loop { agents [...] max_iterations N until ... }` |
| Until (verify) | `until { compile true test true impact_tests true }` |
| Until (agent) | `until agent <name>` |
| Until (command) | `until command "shell command"` |

---

## Common errors

| Error | Cause |
|---|---|
| `undefined client 'foo'` | Agent references a client that is not declared |
| `agent 'a' depends_on 'b' which is not defined` | Unknown agent in `depends_on` |
| `dependency cycle detected: a -> b -> a` | Circular dependency |
| `multiple workflows defined (x, y); pass --workflow <name>` | Ambiguous workflow selection |
| `workflow 'w' has no 'steps' field` | Workflow must list agents |

---

## Rust API

```rust
use gaviero_dsl::compile;

let source = std::fs::read_to_string("workflow.gaviero")?;
let plan = compile(&source, "workflow.gaviero", None, Some("add error handling"))?;

// plan: gaviero_core::swarm::plan::CompiledPlan
// pass to gaviero_core::swarm::pipeline::execute()
```

The `runtime_prompt` argument substitutes `{{PROMPT}}` in all agent prompts and descriptions. Pass `None` if prompts are fully self-contained.
