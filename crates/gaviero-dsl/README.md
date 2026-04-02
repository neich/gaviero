# gaviero-dsl

A declarative language for composing multi-agent AI workflows. You write `.gaviero` files describing `client` backends, `agent` tasks, and `workflow` orchestration; the compiler produces a sequence of `WorkUnit` structures that Gaviero's swarm engine executes.

`.gaviero` files are also the output format of coordinated planning. When you run `/cswarm <task>` in the TUI or `gaviero-cli --coordinated --task "..."`, Opus decomposes the task into a `.gaviero` file saved to `tmp/` for your review before any agents run.

---

## Quick example

```gaviero
client opus {
    tier coordinator
    model "claude-opus-4-6"
    privacy public
}

client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

agent researcher {
    description "Document the architecture"
    client opus
    scope {
        owned    ["docs/architecture.md"]
        read_only ["src/"]
    }
    prompt #"
        Read all source files in src/ and write a comprehensive architecture
        document to docs/architecture.md. Cover module structure, data flow,
        and key abstractions.
    "#
}

agent implementer {
    description "Build the feature described in the architecture doc"
    client sonnet
    depends_on [researcher]
    scope {
        owned    ["src/feature/"]
        read_only ["docs/architecture.md"]
    }
    prompt #"
        Read docs/architecture.md and implement the feature it describes.
        Follow existing patterns. Run cargo check after each change.
    "#
    max_retries 2
}

workflow document_then_build {
    steps [researcher implementer]
}
```

`implementer` starts only after `researcher` completes successfully. See [Running a workflow](#running-a-workflow) for how to execute this file.

---

## Coordinated planning — AI-generated workflows

The easiest way to get a `.gaviero` file is to let Opus write it for you. Describe the task in plain language; Opus decomposes it into agents with explicit scopes, dependencies, and model assignments. The result is saved to `tmp/` for you to inspect and edit before any agent runs.

### TUI

```
/cswarm Add OAuth2 login: design the API, implement handlers, write tests
```

Opus generates the workflow, saves it to `tmp/gaviero_plan_<timestamp>.gaviero`, and opens the file in the editor. Review the plan — check that scopes make sense, that files annotated `// (will be created)` are intentional, edit any agent prompts — then run it:

```
/run tmp/gaviero_plan_1234567890.gaviero
```

### CLI

```
gaviero-cli --repo . --coordinated \
  --task "Add OAuth2 login: design the API, implement handlers, write tests"
```

Outputs the plan path to stdout and instructions to stderr:

```
[mode] coordinated — planning DSL (Opus)
[plan] saved to tmp/gaviero_plan_1234567890.gaviero
[plan] review it, then run with:
         gaviero --script tmp/gaviero_plan_1234567890.gaviero
tmp/gaviero_plan_1234567890.gaviero
```

### Why review before running?

When Opus plans a task it may reference files that don't exist yet — files agents will create during execution. In the generated DSL these are annotated with `// (will be created)`:

```gaviero
agent implement_cpu_sim {
    scope {
        owned ["src/cpu_sim/simulator_cpu.hpp"]  // (will be created)
    }
    ...
}
```

Seeing this before dispatch lets you confirm the path is correct, fix it, or restructure dependencies — instead of discovering the problem as a silent agent failure mid-run.

---

## Running a workflow

### CLI

```
gaviero-cli --repo /path/to/project --script workflows/my_workflow.gaviero
```

| Flag | Default | Description |
|---|---|---|
| `--repo PATH` | `.` | Workspace root |
| `--script PATH` | — | Path to the `.gaviero` file |
| `--auto-accept` | off | Skip interactive review of proposed changes |
| `--max-parallel N` | `1` | Maximum concurrent agents (overridden by `max_parallel` in the workflow block) |
| `--model STRING` | `"sonnet"` | Default model when an agent has no `client` |
| `--namespace STRING` | — | Override the default write namespace |
| `--read-ns STRING` | — | Extra namespace(s) to read from (repeatable) |

### TUI

```
/run workflows/my_workflow.gaviero
```

Paths are resolved relative to the open workspace root; absolute paths are also accepted. The swarm dashboard (`Alt+W`) shows real-time agent status.

---

## The `client` declaration

A `client` declares an LLM backend configuration that agents reference by name.

```gaviero
client <name> {
    tier  coordinator | reasoning | execution | mechanical
    model "<model-string>"
    privacy public | local_only
}
```

| Field | Required | Default | Description |
|---|---|---|---|
| `tier` | no | `execution` | Routing hint (see below) |
| `model` | no | system default | Model identifier, e.g. `"claude-opus-4-6"` |
| `privacy` | no | `public` | Data handling level (`public` or `local_only`) |

### Tier

`tier` is a routing hint — it tells the swarm engine the intended capability level, not a hard constraint. Any model can be assigned any tier.

| Tier | Typical use |
|---|---|
| `coordinator` | Planning, analysis, design decisions — Opus-class models |
| `reasoning` | Deep multi-file reasoning tasks |
| `execution` | Focused implementation, fixes — Sonnet-class models |
| `mechanical` | Fast, repetitive, low-complexity tasks — Haiku-class or local models |

### Privacy

`local_only` tags memory writes produced by this agent so that agents using `public`-privacy clients will not receive those memories when reading the same namespace. It has no effect on filesystem writes.

---

## The `agent` declaration

An `agent` declares one task unit — a prompt, a file scope, and optional execution constraints.

```gaviero
agent <name> {
    description "<text>"
    client <client-name>
    scope {
        owned    ["path/" ...]
        read_only ["path/" ...]
    }
    prompt #" multiline text "#
    depends_on [<agent-name> ...]
    max_retries <integer>
    memory {
        read_ns           ["ns1" "ns2"]
        write_ns          "ns"
        importance        <0.0–1.0>
        staleness_sources ["path/" ...]
    }
}
```

### `description`

Short label used for display and logging. Optional; defaults to the agent's name.

### `client`

References a `client` declaration by name. If omitted, the system default model and tier are used.

### `scope`

Controls which files the agent may read and write.

- `owned` — paths the agent may read and write. Proposed changes are restricted to these paths.
- `read_only` — paths the agent may read but not write. Useful for passing context (docs, configs) without allowing modification.
- If `scope` is omitted entirely, the agent defaults to `owned ["."]` — full workspace write access.

Paths are relative to the workspace root. Glob patterns are accepted.

**Scope in generated plans:** When Opus generates a `.gaviero` file, each agent's `owned` paths are guaranteed to be disjoint from every other agent's `owned` paths. A file annotated `// (will be created)` does not exist yet in the workspace — it will be created by the agent during execution.

### `prompt`

The instruction given to the agent.

Use raw strings (`#"..."#`) for multi-line prompts — they span multiple lines, leading and trailing whitespace is trimmed, and backslashes are literal (no escape processing). Use regular double-quoted strings for short single-line prompts.

```gaviero
// short
prompt "Append a changelog entry."

// multi-line
prompt #"
    Analyze src/ for security issues.
    Write findings to docs/findings.md.
"#
```

### `depends_on`

A list of agent names that must complete successfully before this agent starts.

```gaviero
depends_on [analyze_coverage analyze_structure]
```

The compiler detects cycles and reports an error with the cycle path. An agent that is not listed in any `depends_on` may run as soon as the concurrency limit permits.

### `max_retries`

How many times to retry the agent on failure. Default is `1` (no retry). `max_retries 3` means up to 3 total attempts.

### `memory { ... }`

See the [Memory](#memory) section.

---

## The `workflow` declaration

A `workflow` declares an execution plan — an ordered list of agents and optional concurrency and memory settings.

```gaviero
workflow <name> {
    steps        [<agent-name> ...]
    max_parallel <integer>
    memory {
        read_ns  ["ns1" ...]
        write_ns "ns"
    }
}
```

| Field | Required | Description |
|---|---|---|
| `steps` | yes | Ordered list of agent names to execute |
| `max_parallel` | no | Maximum number of agents that may run concurrently |
| `memory` | no | Workflow-level memory defaults inherited by all agents |

`max_parallel` overrides the `--max-parallel` CLI flag when both are specified.

### Workflow selection

| Situation | Behavior |
|---|---|
| Exactly one `workflow` in the file | Used automatically |
| No `workflow` declarations | All `agent` declarations run in source order with no concurrency limit |
| More than one `workflow` in the file | Compile error — split into separate files |

---

## Memory

### What it does

Gaviero maintains a persistent semantic memory store across runs. When an agent completes, its results can be written to a named namespace in this store. On the next run — or for downstream agents in the same run — those namespaces are searched semantically and relevant context is injected into the agent's working memory automatically. This lets agents build on prior work without repeating expensive analysis.

### Namespaces

Namespaces are string identifiers that group related memories. They are created automatically on first write; choose names that reflect the content semantically.

### Where `memory { }` can appear

Both `agent` and `workflow` blocks accept a `memory { }` block. The workflow block sets defaults that all agents inherit. Agent blocks extend or override those defaults.

### Fields

| Field | Valid in | Type | Description |
|---|---|---|---|
| `read_ns` | agent, workflow | string list | Namespaces to search when building this agent's context |
| `write_ns` | agent, workflow | string | Namespace where this agent's results are stored |
| `importance` | **agent only** | float 0.0–1.0 | Retrieval weight for memories written by this agent. Default: `0.5` |
| `staleness_sources` | **agent only** | string list | File paths; if any have changed since last run, cached memory is invalidated |

### Merge rules

These rules apply when both the workflow and agent declare memory fields:

- **`read_ns` is additive.** The workflow's namespaces come first, the agent's namespaces are appended, and duplicates are removed (order preserved). An agent therefore always reads from all workflow namespaces plus its own.
- **`write_ns` is override.** An agent's `write_ns` replaces the workflow's. If the agent omits it, the workflow's value is used. If neither declares it, no memory writes occur.
- **`importance` and `staleness_sources` are agent-only.** There are no workflow-level defaults for these fields.

Example: with workflow `read_ns ["shared" "policies"]` and agent `read_ns ["prior-results"]`, the agent reads from `["shared" "policies" "prior-results"]`.

### Staleness invalidation

When `staleness_sources` lists file paths, the system computes file hashes before the agent runs. If any listed file has changed since the agent last produced output, the cached memory for that agent is discarded and the agent runs fresh. Use this when an agent's output depends on source files that change between runs.

### Privacy and memory

When a `local_only` client writes memories, those entries are tagged as private. When an agent using a `public` client reads the same namespace via `read_ns`, the private entries are filtered out. This lets a local model perform sensitive pre-processing while keeping its outputs invisible to cloud-hosted agents.

---

## Examples

### 1. Single agent — generate API documentation

No `workflow` is needed; a single agent runs automatically.

```gaviero
client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

agent generate_docs {
    description "Generate API documentation from source"
    client sonnet
    scope {
        owned    ["docs/api.md"]
        read_only ["src/"]
    }
    prompt #"
        Read all public functions and types in src/.
        Write a Markdown API reference to docs/api.md.
        Include: function signatures, parameter descriptions, return values,
        and one usage example per function.
    "#
}
```

---

### 2. Sequential pipeline — two-stage code review

`report` waits for `analyze` before starting. If `analyze` fails, `report` is skipped.

```gaviero
client opus {
    tier coordinator
    model "claude-opus-4-6"
    privacy public
}

client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

agent analyze {
    description "Identify code quality issues"
    client opus
    scope {
        owned    ["docs/analysis.md"]
        read_only ["src/"]
    }
    prompt #"
        Review src/ for code quality issues: long functions, missing error
        handling, unclear naming, missing tests. Write findings to
        docs/analysis.md with severity ratings (high/medium/low).
    "#
}

agent report {
    description "Write an executive summary from the analysis"
    client sonnet
    depends_on [analyze]
    scope {
        owned    ["docs/report.md"]
        read_only ["docs/analysis.md"]
    }
    prompt #"
        Read docs/analysis.md. Write docs/report.md: a short executive
        summary listing the top 5 issues, estimated effort to fix each,
        and a recommended prioritization order.
    "#
}

workflow code_review {
    steps [analyze report]
}
```

---

### 3. Parallel execution — monorepo performance audit

`analyze_backend` and `analyze_frontend` run concurrently (`max_parallel 2`). `cross_cutting_report` waits for both before starting — a fan-in pattern.

```gaviero
client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

agent analyze_backend {
    description "Audit backend performance bottlenecks"
    client sonnet
    scope {
        owned    ["docs/backend_audit.md"]
        read_only ["backend/src/"]
    }
    prompt #"
        Profile the backend API handlers for performance issues.
        Write docs/backend_audit.md with findings and severity ratings.
    "#
}

agent analyze_frontend {
    description "Audit frontend bundle size and render performance"
    client sonnet
    scope {
        owned    ["docs/frontend_audit.md"]
        read_only ["frontend/src/"]
    }
    prompt #"
        Analyze the frontend bundle configuration and component render costs.
        Write docs/frontend_audit.md with findings and severity ratings.
    "#
}

agent cross_cutting_report {
    description "Synthesize both audits into a unified report"
    client sonnet
    depends_on [analyze_backend analyze_frontend]
    scope {
        owned    ["docs/performance_report.md"]
        read_only ["docs/backend_audit.md" "docs/frontend_audit.md"]
    }
    prompt #"
        Read both audit documents. Write docs/performance_report.md with
        the top 10 issues across frontend and backend, prioritized by impact.
        Note any cross-cutting concerns that affect both layers.
    "#
}

workflow performance_audit {
    steps [analyze_backend analyze_frontend cross_cutting_report]
    max_parallel 2
}
```

---

### 4. Memory-enabled workflow — codebase health monitor

On the first run, `survey` finds no prior history. On subsequent runs, the memory context includes past findings, so the agent can report trends and regressions. `importance 0.8` means survey results rank highly in semantic retrieval.

```gaviero
client opus {
    tier coordinator
    model "claude-opus-4-6"
    privacy public
}

client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

agent survey {
    description "Survey codebase health and record findings"
    client opus
    scope {
        owned    ["docs/health.md"]
        read_only ["src/"]
    }
    memory {
        read_ns  ["health-history"]   // read prior survey results on every run
        write_ns "health-history"     // write this run's findings back to the same namespace
        importance 0.8
    }
    prompt #"
        Read the memory context for any prior health survey results.
        Survey the codebase for: test coverage gaps, TODO/FIXME comments,
        dead code, and outdated dependencies.
        Write docs/health.md comparing current state to prior findings.
        Note improvements and regressions since the last survey.
    "#
}

agent remediate {
    description "Fix the top issue identified by the survey"
    client sonnet
    depends_on [survey]
    scope {
        owned    ["src/"]
        read_only ["docs/health.md"]
    }
    memory {
        read_ns  ["health-history"]   // understand prior context before acting
        write_ns "remediation-log"    // record what was changed and why
        importance 0.6
    }
    prompt #"
        Read the survey in docs/health.md and the memory context.
        Fix the single highest-priority issue found.
        Record what was changed and why in the memory context.
    "#
}

workflow health_monitor {
    steps [survey remediate]
}
```

---

### 5. Advanced memory — security audit pipeline

Demonstrates workflow-level memory defaults, per-agent namespace overrides, staleness invalidation, importance weighting, and `local_only` privacy filtering.

The workflow declares baseline `read_ns` namespaces inherited by every agent. Each agent overrides `write_ns` to produce a dedicated namespace. Downstream agents accumulate `read_ns` to see the full upstream audit trail.

```gaviero
client opus {
    tier coordinator
    model "claude-opus-4-6"
    privacy public
}

client sonnet {
    tier reasoning
    model "claude-sonnet-4-6"
    privacy public
}

client haiku {
    tier execution
    model "claude-haiku-4-5-20251001"
    privacy public
}

// local_only: results written by agents using this client are invisible
// to agents with public-privacy clients reading the same namespace.
client local {
    tier mechanical
    privacy local_only
}

agent scan {
    description "Scan codebase for security vulnerabilities"
    client opus
    scope {
        owned    ["docs/security_findings.md"]
        read_only ["src/" "Cargo.toml" "Cargo.lock"]
    }
    memory {
        // Effective read_ns: ["shared" "security-policies" "prior-audits"]
        // (workflow namespaces first, then agent-specific)
        read_ns           ["prior-audits"]
        write_ns          "scan-findings"   // overrides workflow default "security-audit"
        importance        0.9               // high: scan results are critical for downstream agents
        staleness_sources ["src/"]          // invalidate cache if src/ changes between runs
    }
    prompt #"
        Scan the codebase for security vulnerabilities. Check for:
        - SQL injection, XSS, CSRF vulnerabilities
        - Insecure dependencies (check Cargo.lock)
        - Hardcoded secrets or credentials
        - Unsafe Rust patterns

        Document all findings in docs/security_findings.md.
        Use memory context for any prior audit results.
    "#
    max_retries 2
}

agent fix {
    description "Fix identified vulnerabilities"
    client sonnet
    depends_on [scan]
    scope {
        owned    ["src/"]
        read_only ["docs/security_findings.md"]
    }
    memory {
        // Effective read_ns: ["shared" "security-policies" "scan-findings"]
        read_ns           ["scan-findings"]
        write_ns          "fix-results"
        importance        0.8
        staleness_sources ["src/"]
    }
    prompt #"
        Fix the security vulnerabilities documented in docs/security_findings.md.
        The memory context contains the scan agent's findings — use it as your
        primary reference. Apply minimal, targeted fixes without refactoring.
    "#
    max_retries 2
}

agent write_tests {
    description "Write security regression tests"
    client haiku
    depends_on [fix]
    scope {
        owned    ["tests/security/"]
        read_only ["src/" "docs/security_findings.md"]
    }
    memory {
        // Effective read_ns: ["shared" "security-policies" "scan-findings" "fix-results"]
        read_ns   ["scan-findings" "fix-results"]
        write_ns  "test-results"
        importance 0.7
    }
    prompt #"
        Write regression tests in tests/security/ for every vulnerability fixed.
        Reference the scan findings and fix results from memory context.
        Each test should verify that the specific vulnerability no longer exists.
    "#
}

agent verify {
    description "Verify all fixes and tests pass"
    client sonnet
    depends_on [write_tests]
    scope {
        read_only ["src/" "tests/" "docs/security_findings.md"]
    }
    memory {
        // Effective read_ns: ["shared" "security-policies" "scan-findings" "fix-results" "test-results"]
        read_ns   ["scan-findings" "fix-results" "test-results"]
        write_ns  "verification-results"
        importance 0.6
    }
    prompt #"
        Verify that:
        1. All vulnerabilities in docs/security_findings.md have been fixed
        2. All regression tests pass (check tests/security/)
        3. No new vulnerabilities were introduced

        Report pass/fail for each finding. Use memory context for the full audit trail.
    "#
    max_retries 2
}

workflow security_audit {
    steps [scan fix write_tests verify]
    max_parallel 2
    memory {
        // All agents inherit these as baseline read namespaces.
        // Each agent's own read_ns is appended to this list.
        read_ns  ["shared" "security-policies"]
        // Default write namespace — overridden per agent above.
        write_ns "security-audit"
    }
}
```

---

### 6. Multi-client tier routing — cost-aware feature addition

Assigns each agent to the cheapest model capable of its task. `design` uses the coordinator tier for complex decision-making; `implement` uses execution for focused coding; `update_changelog` uses mechanical for a simple repetitive task.

```gaviero
client opus {
    tier coordinator
    model "claude-opus-4-6"
    privacy public
}

client sonnet {
    tier execution
    model "claude-sonnet-4-6"
    privacy public
}

client haiku {
    tier mechanical
    model "claude-haiku-4-5-20251001"
    privacy public
}

agent design {
    description "Design the feature architecture and public API"
    client opus                        // coordinator: best for design decisions
    scope {
        owned    ["docs/design.md"]
        read_only ["src/" "tests/"]
    }
    prompt #"
        Design the new feature. Define the public API, data structures,
        and module boundaries. Write docs/design.md covering: interface
        contract, data flow, and integration points with existing modules.
    "#
}

agent implement {
    description "Implement the feature"
    client sonnet                      // execution: capable and cost-efficient
    depends_on [design]
    scope {
        owned    ["src/feature/"]
        read_only ["docs/design.md" "tests/"]
    }
    prompt #"
        Implement the feature according to docs/design.md.
        Follow existing code patterns. Run cargo check after each change.
    "#
    max_retries 2
}

agent update_changelog {
    description "Append a changelog entry"
    client haiku                       // mechanical: simple repetitive task — cheapest tier
    depends_on [implement]
    scope {
        owned    ["CHANGELOG.md"]
        read_only ["docs/design.md"]
    }
    prompt #"
        Append a new entry to CHANGELOG.md describing the feature from
        docs/design.md. Follow the existing changelog format exactly.
    "#
}

workflow add_feature {
    steps [design implement update_changelog]
}
```

---

## Language reference

### Grammar at a glance

```gaviero
// client — LLM backend configuration
client <name> {
    tier    coordinator | reasoning | execution | mechanical
    model   "<model-string>"
    privacy public | local_only
}

// agent — one task unit
agent <name> {
    description "<text>"
    client      <client-name>
    scope {
        owned    ["path/" ...]          // writable paths (default if omitted: ["."])
        read_only ["path/" ...]
    }
    prompt #" multiline text "#         // or "single-line string"
    depends_on   [<agent-name> ...]
    max_retries  <integer>              // default: 1
    memory {
        read_ns           ["ns1" ...]   // additive merge with workflow read_ns
        write_ns          "ns"          // overrides workflow write_ns
        importance        <0.0–1.0>     // default: 0.5  — agent only
        staleness_sources ["path/" ...] // agent only
    }
}

// workflow — execution plan
workflow <name> {
    steps        [<agent-name> ...]
    max_parallel <integer>
    memory {
        read_ns  ["ns1" ...]            // inherited by all agents (prepended)
        write_ns "ns"                   // default for agents that omit write_ns
    }
}
```

### Comments

`// line comments` are supported anywhere. There are no block comments.

### String types

| Syntax | Use | Notes |
|---|---|---|
| `"text"` | Short single-line values | Standard escaped string |
| `#"text"#` | Multi-line prompts | No escape processing; leading/trailing whitespace trimmed |

### Identifiers

May contain letters, digits, underscores, and hyphens. Examples: `my-agent`, `scan_v2`, `claude-opus-4-6`. Lists of identifiers or strings are space-separated inside `[...]` — no commas.

---

## Error messages

| Message | Cause | Fix |
|---|---|---|
| `duplicate client name 'foo'` | Two `client` blocks share a name | Rename one |
| `undefined client 'foo'` | Agent references an undeclared client | Declare the client or fix the name |
| `workflow step 'foo' is not a defined agent` | A name in `steps [...]` has no matching `agent` | Fix the name or declare the agent |
| `agent 'a' depends_on 'ghost' which is not defined` | A name in `depends_on [...]` has no matching `agent` | Fix the name or declare the agent |
| `dependency cycle detected: a -> b -> a` | A `depends_on` chain forms a loop | Break the cycle by removing one dependency |
| `multiple workflows defined` | More than one `workflow` block in the file | Keep one workflow per file, or split into separate files |
| `workflow 'foo' has no 'steps' field` | A `workflow` block was declared without `steps` | Add `steps [...]` |

Error output uses colorized source diagnostics pointing to the exact line and column in the file.

---

## Bundled examples

The `examples/` directory contains ready-to-run workflows demonstrating the full DSL:

| File | What it demonstrates |
|---|---|
| `bugfix_with_tests.gaviero` | 3-stage pipeline: diagnose → fix → verify. Strict scoping and retry logic |
| `feature_tdd.gaviero` | TDD workflow: write tests first, implement, then verify no regressions |
| `multi_crate_test.gaviero` | Cross-crate monorepo changes with workspace-level test verification |
| `refactor_safe.gaviero` | Parallel coverage + structure analysis feeding a refactor agent; `max_parallel` |
| `security_audit.gaviero` | 4-stage security scan: scan → fix → write tests → verify |
| `security_audit_memory.gaviero` | Full memory system: workflow defaults, per-agent overrides, staleness, privacy |

Run any example against your own project:

```
gaviero-cli --repo /path/to/your/project --script examples/bugfix_with_tests.gaviero
```

Or from the TUI (workspace must be open):

```
/run examples/bugfix_with_tests.gaviero
```
