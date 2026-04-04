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
        owned    ["src/" "tests/"]
        read_only ["docs/"]
    }
    prompt "Fix every issue in the reviewer's list. Do not change anything else."
    max_retries 3
}

workflow review_and_fix {
    steps [reviewer fixer]
    verify { compile true clippy true }
}
```

Run with `gaviero-cli --script review_and_fix.gaviero` or `/run review_and_fix.gaviero` in the TUI.

---

## Complete workflow examples

### 1. Bug fix with TDD — `bugfix_with_tests.gaviero`

The canonical single-agent workflow. The agent generates failing tests first, then fixes the code until they pass. Use this for any well-scoped bug.

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

The `{{PROMPT}}` placeholder is replaced with the `--task` value at compile time. The agent generates a failing test first, then iterates until `cargo test` passes.

---

### 2. New feature with test-first TDD — `feature_tdd.gaviero`

For larger features: Opus writes the specification as failing tests; Haiku implements until they pass; a final sonnet-tier agent validates edge cases.

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

        Requirements:
        - Tests MUST fail against the current code (they describe desired state)
        - Tests MUST compile
        - Cover: happy path, edge cases, error cases
        - Use the project's existing test framework
        - Do NOT modify any source files
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
        compile true
        clippy  true
        test    true
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

When the correct refactoring approach is ambiguous, generate three independent attempts and keep the one that passes all gates. Cheap to run; the engine picks the winner automatically.

```gaviero
client haiku { tier cheap model "claude-haiku-4-5-20251001" }

agent refactor {
    description "Refactor {{PROMPT}}"
    client haiku
    scope {
        owned    ["src/core/"]
        read_only ["tests/core/" "src/types.rs"]
    }
    prompt #"
        Refactor the following: {{PROMPT}}

        Constraints:
        - All existing tests must pass unchanged
        - No change to public API signatures
        - No change to error types
        - Apply Rust idiomatic patterns (iterators, ? operator, etc.)
    "#
    max_retries 4
}

workflow refactor_safe {
    steps          [refactor]
    strategy       best_of_3       // 3 independent attempts
    max_retries    4               // retries per attempt
    escalate_after 2               // switch to expensive after 2 failed retries

    verify {
        compile true
        clippy  true
        test    true
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

Explicit multi-agent workflow. Each agent owns a disjoint scope; the integration agent runs after both finish.

```gaviero
client haiku  { tier cheap     model "claude-haiku-4-5-20251001" }
client sonnet { tier cheap     model "claude-sonnet-4-6" }

// Two parallel agents — independent scopes, no conflict
agent core-types {
    description "Rename ModelTier variants and update all usages in gaviero-core"
    client haiku
    scope {
        owned ["crates/gaviero-core/src/"]
    }
    prompt #"
        Rename the ModelTier enum variants: Execution → Cheap, Coordinator → Expensive.
        Update every match arm, struct field, and test that references the old names.
        The enum definition is in src/types.rs.
    "#
    max_retries 3
}

agent dsl-update {
    description "Update DSL compiler to emit new ModelTier values"
    client haiku
    scope {
        owned    ["crates/gaviero-dsl/src/"]
        read_only ["crates/gaviero-core/src/types.rs"]
    }
    prompt #"
        Update the DSL compiler (src/compiler.rs) to map tier literals to the
        renamed ModelTier variants: Cheap and Expensive.
        The new enum definition will be in gaviero-core/src/types.rs.
    "#
    max_retries 3
}

// Integration agent runs after both complete
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
        Run cargo check workspace and fix any remaining compilation errors.
        Do not change logic — only fix name references.
    "#
    max_retries 5
}

workflow multi_crate_rename {
    steps        [core-types dsl-update integration-check]
    max_parallel 2          // core-types and dsl-update run concurrently
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

Read-only audit that writes its findings to a markdown report and to memory. No source files modified.

```gaviero
client opus { tier expensive model "claude-opus-4-6" }

agent audit {
    description "Security audit of the authentication layer"
    client opus
    scope {
        owned    ["docs/security-audit.md"]    // only the report is writable
        read_only ["src/auth/" "src/middleware/" "tests/"]
    }
    prompt #"
        Perform a security audit of the authentication implementation.

        Review for:
        - Injection vulnerabilities (SQL, command, path traversal)
        - Authentication bypass risks
        - Session management issues
        - Insecure defaults or hardcoded secrets
        - Missing input validation
        - Timing attacks

        Write findings to docs/security-audit.md using the format:
        ## [CRITICAL|HIGH|MEDIUM|LOW] <title>
        **File:** path/to/file.rs:line
        **Description:** ...
        **Recommendation:** ...
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
    strategy single_pass    // one thorough pass is enough
}
```

Run:
```bash
gaviero-cli --script security_audit.gaviero --model opus
```

The audit findings are stored to the `security-audits` memory namespace so future agents (and future audit runs) can access prior findings.

---

### 6. Memory-accumulating multi-run workflow — `security_audit_memory.gaviero`

On first run the agent audits and stores findings. On subsequent runs it reads prior findings and focuses on new code paths.

```gaviero
client opus   { tier expensive model "claude-opus-4-6" }
client sonnet { tier cheap     model "claude-sonnet-4-6" }

agent triage {
    description "Triage new code changes against prior audit findings"
    client sonnet
    scope {
        read_only ["src/"]
    }
    prompt #"
        Given the prior security audit findings in memory, identify which
        recent changes to src/ introduce new risk or touch previously flagged areas.
        Output a prioritised list of files to re-audit.
    "#
    max_retries 1

    memory {
        read_ns  ["security-audits"]
        write_ns "security-triage"
    }
}

agent deep-audit {
    description "Deep audit of prioritised files"
    client opus
    depends_on [triage]
    scope {
        owned    ["docs/security-audit.md"]
        read_only ["src/"]
    }
    prompt #"
        Using the triage output, perform a deep security audit of the
        prioritised files. Update docs/security-audit.md with new findings.
        Prefix each finding with [NEW] or [UPDATED] if it supersedes a prior finding.
    "#
    max_retries 2

    memory {
        read_ns           ["security-audits" "security-triage"]
        write_ns          "security-audits"
        importance        0.9
        staleness_sources ["src/auth/" "src/api/"]
    }
}

workflow incremental_audit {
    steps    [triage deep-audit]
    strategy refine
}
```

---

### 7. Local-only (no API) workflow — `local_refactor.gaviero`

Uses Ollama for privacy-sensitive code. No data leaves the machine.

```gaviero
client local {
    tier     cheap
    model    "qwen2.5-coder:14b"
    privacy  local_only          // forces Ollama, rejects any API backend
}

agent refactor {
    description "Refactor {{PROMPT}} using local model"
    client local
    scope {
        owned ["src/internal/"]
    }
    prompt "Refactor: {{PROMPT}}. Keep the public API unchanged."
    max_retries 3
}

workflow local_refactor {
    steps    [refactor]
    strategy refine
    verify   { compile true }
}
```

Run:
```bash
gaviero-cli --script local_refactor.gaviero \
  --task "extract the connection pool into its own module"
```

Requires a running Ollama instance (`ollama serve`).

---

## Declarations

### `client` — LLM backend configuration

```gaviero
client <name> {
    tier  cheap | expensive          // model tier (default: cheap)
    model "<model-id>"               // override specific model
    privacy public | local_only      // public: any API; local_only: Ollama only
}
```

**Tier values:**

| Value | Maps to | Typical model |
|---|---|---|
| `cheap` | `ModelTier::Cheap` | claude-haiku-4-5 |
| `expensive` | `ModelTier::Expensive` | claude-sonnet-4-6 |
| `coordinator`, `reasoning` | `Expensive` (deprecated) | |
| `execution`, `mechanical` | `Cheap` (deprecated) | |

### `agent` — Work unit

```gaviero
agent <name> {
    description "<task description>"          // shown in UI; {{PROMPT}} substitution
    client      <client-name>
    scope {
        owned    ["path/" "file.rs"]          // agent can read and write these
        read_only ["docs/" "other.md"]        // agent can read but not write
    }
    depends_on  [agent-a agent-b]             // wait for these to complete first
    prompt #"
        Multi-line raw string instructions.
        {{PROMPT}} is replaced with the --task / runtime_prompt value.
    "#
    max_retries 3                             // inner validation-feedback cycles (default: 1)
    memory {
        read_ns           ["ns-1" "ns-2"]     // namespaces to search at run time
        write_ns          "my-ns"             // namespace for storing results
        importance        0.8                 // memory importance weight (0.0–1.0)
        staleness_sources ["src/"]            // invalidate memory if these paths change
    }
}
```

If `scope` is omitted the agent gets `owned = ["."]` (full workspace).

### `workflow` — Orchestration

```gaviero
workflow <name> {
    steps        [agent-a agent-b agent-c]    // execution order (respects depends_on)
    max_parallel 2                            // max concurrent agents (default: 1)

    // Iteration strategy
    strategy     single_pass | refine | best_of_N   // default: refine
    max_retries  5                            // inner retries per attempt (default: 5)
    attempts     3                            // outer attempts for BestOfN (default: 1)
    test_first   true                         // generate failing tests before editing
    escalate_after 2                          // switch to expensive model after N failures

    // Post-edit verification
    verify {
        compile true      // run cargo check / equivalent
        clippy  true      // run cargo clippy
        test    false     // run test suite
    }

    // Workflow-level memory defaults (merged with agent-level settings)
    memory {
        read_ns  ["shared" "policies"]
        write_ns "default-ns"
    }
}
```

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

## Memory system

### Namespace merge rules

| Field | Rule |
|---|---|
| `read_ns` | Additive: workflow list + agent list, deduplicated |
| `write_ns` | Agent value overrides workflow value |
| `importance` | Agent-only |
| `staleness_sources` | Agent-only |

### Staleness

If `staleness_sources` lists a path (e.g., `"src/auth/"`), memory entries stored from a previous run are automatically invalidated when those files have changed since the entry was written. This prevents stale context from polluting future runs.

---

## Execution without a workflow

If the file contains **no workflow declaration**, all agents run in declaration order.

If the file contains **exactly one workflow**, it is used automatically.

If the file contains **multiple workflows**, pass `--workflow <name>` (CLI) to select one.

---

## `{{PROMPT}}` substitution

Any `description` or `prompt` field may contain `{{PROMPT}}`. This is replaced at compile time by the `runtime_prompt` argument — the value of `--task` on the CLI, or the message sent to `/run` in the TUI.

This lets a single `.gaviero` file act as a reusable template:

```bash
# Same script, different task each time
gaviero-cli --script bugfix.gaviero --task "null pointer in UserService.getById()"
gaviero-cli --script bugfix.gaviero --task "race condition in connection pool shutdown"
```

---

## Dependency edges

`depends_on` creates a directed edge in the execution DAG. Agents with no dependencies run in parallel (subject to `max_parallel`). Cycles are detected at compile time.

An agent is skipped if any of its dependencies failed (trigger rule: `AllSuccess`). Skipped agents appear in the result with status `Skipped`.

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
