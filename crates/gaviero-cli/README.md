# gaviero-cli

Headless AI agent task runner for Gaviero. Executes agent workflows against a local repository from the command line — no TUI required. Suitable for CI pipelines, scripted automation, and one-off tasks.

---

## Synopsis

```
gaviero-cli [OPTIONS] --task <TEXT>
gaviero-cli [OPTIONS] --script <FILE.gaviero>
gaviero-cli [OPTIONS] --work-units <JSON>
```

---

## Flags reference

| Flag | Type | Default | Description |
|---|---|---|---|
| `--repo PATH` | path | `.` | Workspace root. All relative file paths in agents are resolved against this directory. |
| `--task TEXT` | string | — | Single task description. Runs one agent with full repo write access. |
| `--script PATH` | file | — | Path to a `.gaviero` DSL script. |
| `--work-units JSON` | JSON | — | JSON array of `WorkUnit` objects (advanced). |
| `--model STRING` | string | `"sonnet"` | Model to use in `--task` mode. Ignored in `--coordinated` and `--script` (model is set per-agent in the DSL). |
| `--auto-accept` | flag | off | Skip interactive review of proposed file changes. |
| `--max-parallel N` | int | `1` | Maximum concurrent agents. Overridden by `max_parallel` if declared in the workflow. |
| `--namespace STRING` | string | — | Write namespace for this run's memory output (see [Memory](#memory)). |
| `--read-ns STRING` | string | — | Extra namespace to read from. Repeatable: `--read-ns ns1 --read-ns ns2`. |
| `--coordinated` | flag | off | Coordinator mode: Opus decomposes the task into a tier-routed DAG. Requires `--task`. |
| `--format text\|json` | string | `text` | Output format for results. |

### Mutual exclusivity

`--task`, `--script`, and `--work-units` are mutually exclusive. Exactly one must be provided.

---

## Input modes

### `--task` — single agent

The simplest mode. Provide a natural-language task description; a single agent is created with full read/write access to `--repo`.

```
gaviero-cli --repo . --task "Add input validation to the login form in src/auth.rs"
```

The agent uses the model specified by `--model` (default: `sonnet`). To use a more capable model:

```
gaviero-cli --repo . --model opus --task "Design the data model for the new billing system"
```

### `--script` — DSL workflow

Run a pre-written `.gaviero` file. The DSL lets you declare multiple agents, file scopes, dependencies, memory blocks, and parallelism. See the [gaviero-dsl README](../gaviero-dsl/README.md) for the full language reference.

```
gaviero-cli --repo . --script workflows/feature_tdd.gaviero
```

The `max_parallel` field in the workflow block overrides `--max-parallel` when both are specified.

### `--work-units` — JSON (advanced)

Pass a raw JSON array of `WorkUnit` objects. This is intended for programmatic invocation when you are generating work units from external tooling.

```
gaviero-cli --repo . --work-units '[
  {
    "id": "wu-1",
    "description": "Refactor the config loader",
    "scope": { "owned_paths": ["src/config/"], "read_only": [] },
    "depends_on": [],
    "model": "sonnet"
  }
]'
```

---

## Output

### Text format (default)

One line per agent with status and modified files:

```
task-0: OK (src/auth.rs, tests/auth_test.rs)
task-1: FAIL: timeout exceeded (src/config.rs)
```

Status values: `OK` on success, `FAIL: <reason>` on failure.

### JSON format

Full machine-readable result including all per-agent details:

```
gaviero-cli --repo . --task "..." --format json
```

```json
{
  "success": true,
  "manifests": [
    {
      "work_unit_id": "task-0",
      "status": "Completed",
      "modified_files": ["src/auth.rs", "tests/auth_test.rs"]
    }
  ]
}
```

Useful for CI pipelines that need to parse results programmatically.

### Stderr diagnostics

Progress information is written to stderr so it does not pollute stdout:

```
[namespace] write=my-project, read=[my-project]
[memory] ready
[phase] running
[agent:task-0] Running "..."
  [tool] write_file
  [done]
[completed] success=true
```

---

## Coordinated mode (`--coordinated`)

Requires `--task`. In coordinated mode, Claude Opus first decomposes the task into a dependency-ordered DAG of work units, each annotated with a model tier. The units are then dispatched to the appropriate model for execution.

```
gaviero-cli --repo . --coordinated \
  --task "Add OAuth2 login flow: design the API, implement the backend handlers,
          write tests, and update the documentation"
```

Typical stderr output:

```
[mode] coordinated (Opus → Sonnet/Haiku tier routing)
[coordinator] planning: Add OAuth2 login flow: design the API, implement t...
[coordinator] planned 4 units: design-api, implement-handlers, write-tests, update-docs
[dispatch] design-api     tier=Planning  backend=opus
[dispatch] implement-handlers  tier=Compute  backend=sonnet
[dispatch] write-tests    tier=Compute  backend=sonnet
[dispatch] update-docs    tier=Mechanical  backend=haiku
[tier] 1/3
...
[completed] success=true
```

The `--model` flag is ignored in coordinated mode — the coordinator always uses Opus, and execution tiers use Sonnet/Haiku as appropriate.

---

## Memory

Gaviero maintains a persistent semantic memory store. Each run can write results to a named **namespace** and read context from one or more namespaces. This lets agents build on prior work across runs.

The CLI prints the active namespace configuration at startup:

```
[namespace] write=my-project, read=[my-project]
[memory] ready
```

### Namespace resolution (priority order)

**Write namespace** (where this run's results are stored):
1. `--namespace` flag
2. Workspace settings
3. Folder name (fallback)

**Read namespaces** (where context is retrieved from):
1. Workspace settings base list
2. `--read-ns` flags (deduplicated)
3. The write namespace is always prepended so the run can read its own prior output

### Accumulating knowledge across runs

Run the same task repeatedly and each run will see the previous run's findings:

```
# First run — memory is empty, agent starts fresh
gaviero-cli --repo . --namespace audit-log \
  --task "Survey the codebase for test coverage gaps. Write a summary to docs/coverage.md."

# Second run — agent reads prior findings, reports on changes
gaviero-cli --repo . --namespace audit-log \
  --task "Survey the codebase for test coverage gaps. Write a summary to docs/coverage.md.
          Compare with prior findings from memory context and note improvements or regressions."

# Third run and beyond — trends accumulate over time
gaviero-cli --repo . --namespace audit-log \
  --task "Continue the coverage survey. Report the trend over the last three surveys."
```

### Reading from multiple namespaces

Use `--read-ns` to pull in context from namespaces written by other workflows:

```
# A security scan that reads both prior security findings and general project context
gaviero-cli --repo . \
  --namespace security-scan \
  --read-ns prior-security-findings \
  --read-ns project-overview \
  --task "Scan src/ for security vulnerabilities. Cross-reference with prior findings in
          memory context. Document new issues and confirm resolved ones."
```

### Namespaces with DSL scripts

When using `--script`, the `.gaviero` file controls namespaces per-agent via `memory {}` blocks. The `--namespace` and `--read-ns` flags act as additional overrides layered on top of the settings baseline, before the script's own namespace declarations take effect.

---

## Examples

### Fix a failing test

```
gaviero-cli --repo . --task "The test auth::tests::login_with_invalid_password is failing.
  Diagnose the root cause and fix it without modifying the test itself."
```

### Generate documentation

```
gaviero-cli --repo . --model sonnet \
  --task "Generate a Markdown API reference for all public functions in src/api/.
          Write it to docs/api_reference.md."
```

### Code review with persistent memory

Each run adds to `code-review` namespace. The agent can track recurring issues over time:

```
gaviero-cli --repo . \
  --namespace code-review \
  --task "Review the changes in the current git diff for: code quality, missing error handling,
          and test coverage. Compare with prior review findings from memory context.
          Write a report to docs/review.md listing new issues and any recurring patterns."
```

### Run a DSL workflow with custom namespace

Override the write namespace at run time without editing the `.gaviero` file:

```
gaviero-cli --repo . \
  --namespace feature-auth-2024 \
  --script workflows/feature_tdd.gaviero
```

### Coordinated task with memory for a large feature

Opus plans the work, execution agents read shared context written by prior runs:

```
gaviero-cli --repo . \
  --coordinated \
  --namespace feature-billing \
  --read-ns project-architecture \
  --task "Implement the billing module: design the data model, implement the service layer,
          write integration tests, and update the API documentation. Read project architecture
          from memory context before starting."
```

The `project-architecture` namespace might have been populated by an earlier architecture survey run.

### Parallel DSL workflow

When a `.gaviero` script declares `max_parallel`, agents that have no mutual dependency run concurrently:

```
gaviero-cli --repo . --script workflows/refactor_safe.gaviero
```

The `max_parallel` value from the workflow block is used automatically. Worktrees are created per agent to prevent filesystem conflicts.

### CI pipeline with JSON output

Capture structured results for downstream processing:

```
result=$(gaviero-cli --repo . --format json --auto-accept \
  --script workflows/security_audit.gaviero)

success=$(echo "$result" | jq '.success')
if [ "$success" != "true" ]; then
  echo "Security audit failed"
  echo "$result" | jq '.manifests[] | select(.status != "Completed")'
  exit 1
fi
```

### Building a multi-run knowledge base

Different workflows each write to their own namespace. A final synthesis step reads from all of them:

```bash
# Step 1: architecture survey
gaviero-cli --repo . --namespace arch-survey \
  --task "Document the overall system architecture, module boundaries, and data flow.
          Write to docs/architecture.md."

# Step 2: security scan (reads architecture context)
gaviero-cli --repo . \
  --namespace security-scan \
  --read-ns arch-survey \
  --task "Scan the codebase for security vulnerabilities. Use the architecture context from
          memory to understand trust boundaries. Write findings to docs/security.md."

# Step 3: performance audit (reads both)
gaviero-cli --repo . \
  --namespace perf-audit \
  --read-ns arch-survey \
  --read-ns security-scan \
  --task "Audit the codebase for performance bottlenecks. Use the architecture and security
          context from memory to prioritize hot paths and avoid touching sensitive areas.
          Write findings to docs/performance.md."

# Step 4: synthesis (reads all three namespaces)
gaviero-cli --repo . \
  --namespace final-report \
  --read-ns arch-survey \
  --read-ns security-scan \
  --read-ns perf-audit \
  --task "Synthesize the architecture, security, and performance findings from memory into
          a single executive report. Highlight the top 10 issues across all dimensions.
          Write to docs/executive_report.md."
```

---

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All agents completed successfully |
| `1` | One or more agents failed, or a startup error occurred |

---

## Notes

- All progress output goes to **stderr**; result output goes to **stdout**. This makes it safe to pipe stdout without mixing diagnostic noise.
- `--auto-accept` skips the interactive file-change review that the TUI normally shows. Use it in CI or when you trust the agent's output.
- The memory store is initialized from `~/.cache/gaviero/`. If initialization fails (missing model files, permissions), the run continues without memory — you will see `[memory] disabled: <reason>` in stderr.
- When `max_parallel > 1` (either from `--max-parallel` or from the workflow's `max_parallel`), each agent runs in a separate git worktree to prevent filesystem conflicts. Worktrees are cleaned up after the run.
