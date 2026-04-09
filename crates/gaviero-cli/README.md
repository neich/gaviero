# gaviero-cli

Headless AI agent task runner. Executes agent workflows against a local repository from the command line — no TUI required. Suitable for CI pipelines, scripted automation, and one-off tasks.

---

## Synopsis

```bash
gaviero-cli [OPTIONS] (--task "<task>" | --work-units '<json>' | --script <file.gaviero>)
```

---

## Input modes

### `--task` — single task

Creates one agent with full workspace scope. The simplest way to run Gaviero. Defaults to `strategy refine` with `max_retries 5`.

```bash
gaviero-cli --task "Add input validation to the login endpoint"
```

### `--script` — DSL workflow file

Compile and execute a `.gaviero` workflow. Supports multi-agent plans, dependencies, memory, graph-context queries, and iteration strategies.

```bash
gaviero-cli --script workflows/refactor.gaviero
```

### `--work-units` — JSON array

Pass `WorkUnit` objects directly. For programmatic callers.

```bash
gaviero-cli --work-units '[{"id":"t1","description":"...","scope":{"owned_paths":["src/"]}}]'
```

---

## All flags

| Flag | Default | Description |
|---|---|---|
| `--repo <path>` | `.` | Workspace root |
| `--model sonnet\|opus\|haiku` | `sonnet` | Model for `--task` mode |
| `--max-parallel <n>` | `1` | Max concurrent agents (enables worktrees when > 1) |
| `--namespace <name>` | (from settings) | Write memory namespace |
| `--read-ns <name>` | (from settings) | Add a read namespace (repeatable) |
| `--format text\|json` | `text` | Output format |
| `--max-retries <n>` | `5` | Inner validation-feedback retries per attempt |
| `--attempts <n>` | `1` | Independent attempts (BestOfN when > 1) |
| `--test-first` | off | Generate failing tests before editing (TDD) |
| `--no-iterate` | off | Single pass only — disables retry loop |
| `--coordinated` | off | Opus planning mode: generate DSL file for review, then exit |
| `--output <path>` | `tmp/gaviero_plan_<ts>.gaviero` | Output path for generated plan (`--coordinated` only) |
| `--resume` | off | Skip already-completed agents from a prior run |
| `--auto-accept` | off | Accept all file changes without review |
| `--trace <file>` | off | Write DEBUG-level JSON trace log |

---

## Use cases

### 1. Quick one-off bug fix

The default mode. The agent iterates with validation feedback until the code compiles and passes lints.

```bash
gaviero-cli --task "Fix the null pointer in UserService.getById when user does not exist"
```

Or with a script that also runs the test suite on each iteration:

```bash
gaviero-cli --script bugfix.gaviero \
  --task "null pointer in UserService.getById when user does not exist"
```

---

### 2. TDD bug fix — generate failing tests first

The `--test-first` flag makes the agent write a test that reproduces the bug before touching any source code. The iteration loop then drives toward the test passing.

```bash
gaviero-cli --task "tokens are not invalidated on logout" \
  --test-first \
  --max-retries 8
```

What happens:
1. Agent writes a test that calls logout and verifies token invalidation — test fails against current code
2. Compiler gate checks the test compiles
3. Agent modifies source code
4. `cargo check` → `cargo clippy` → `cargo test` run after each edit
5. On failure the error is fed back; agent retries up to 8 times

---

### 3. Best-of-N sampling for risky changes

Run multiple independent attempts and keep the one that passes all verification gates. Good when there are several valid refactoring approaches and you want the engine to pick the best.

```bash
# 3 independent attempts; return first to pass all gates (or best by file count)
gaviero-cli --task "Replace the manual error mapping in the parser with thiserror" \
  --attempts 3 \
  --max-retries 4
```

With an explicit script that also enables test verification:

```bash
gaviero-cli --script refactor_safe.gaviero \
  --task "extract connection pool into its own module" \
  --attempts 3
```

---

### 4. New feature with full TDD pipeline

Combine `--test-first` with a liberal retry budget and test verification:

```bash
gaviero-cli --task "Add subscription billing with proration support" \
  --test-first \
  --max-retries 10 \
  --attempts 2 \
  --model haiku
```

Or with a tailored script that uses a spec agent + implement agent:

```bash
gaviero-cli --script feature_tdd.gaviero \
  --task "subscription billing with proration support"
```

---

### 5. Read-only analysis or security audit

Use `--no-iterate` for analysis tasks where the agent writes a report rather than modifying source.

```bash
gaviero-cli --task "Audit the authentication layer for injection vulnerabilities. Write findings to docs/security-audit.md" \
  --no-iterate \
  --model opus
```

Or with a dedicated script that constrains scope and queries the code graph:

```bash
gaviero-cli --script security_audit.gaviero
```

---

### 6. Coordinated multi-agent planning

For large tasks where you want to review the agent decomposition before anything runs:

```bash
# Step 1: Opus decomposes the task into a .gaviero plan
plan=$(gaviero-cli --coordinated \
  --task "Migrate the authentication layer from JWT to session-based auth")

# Step 2: Review the plan
cat "$plan"
$EDITOR "$plan"

# Step 3: Execute when satisfied
gaviero-cli --script "$plan" --max-parallel 3
```

`stdout` prints only the plan path, so the `$()` capture works cleanly in scripts. Use `--output` to control where the plan is written:

```bash
gaviero-cli --coordinated \
  --task "Refactor the billing module" \
  --output plans/billing_refactor.gaviero
```

---

### 7. CI pipeline integration

```bash
#!/bin/bash
set -euo pipefail

result=$(gaviero-cli \
  --script ci_fix.gaviero \
  --task "${CI_TASK:-}" \
  --format json \
  --trace ci-trace-$(date +%s).json)

echo "$result" | jq -r '.manifests[] | "\(.work_unit_id): \(.status)"'

success=$(echo "$result" | jq -r '.success')
if [ "$success" != "true" ]; then
  echo "Gaviero: one or more agents failed" >&2
  exit 1
fi
```

A suitable `ci_fix.gaviero`:

```gaviero
client haiku { tier cheap model "claude-haiku-4-5-20251001" }

agent fix {
    description "{{PROMPT}}"
    client haiku
    scope { owned ["."] }
    prompt "{{PROMPT}}"
    max_retries 5
}

workflow ci {
    steps    [fix]
    strategy refine
    verify   { compile true clippy true test true }
}
```

---

### 8. Resuming an interrupted run

Large multi-agent scripts checkpoint after every completed agent. If the run is interrupted, restart with `--resume`:

```bash
# First run — interrupted after 2/5 agents
gaviero-cli --script big_plan.gaviero --max-parallel 3

# Resume — skips the 2 already-completed agents
gaviero-cli --script big_plan.gaviero --max-parallel 3 --resume
```

Checkpoints are stored in `.gaviero/state/<plan-hash>.json`.

---

### 9. Local-only mode (no API, Ollama)

For privacy-sensitive code that must not leave the machine:

```bash
# Requires a running Ollama instance and a local_only client in the script
gaviero-cli --script local_refactor.gaviero \
  --task "extract the database connection pool" \
  --repo /path/to/private/repo
```

Use `privacy local_only` in the client declaration to prevent the engine from falling back to an API model.

---

### 10. Memory-assisted repeated runs

Run the same task repeatedly over time. Each run stores its results; subsequent runs read prior context automatically.

```bash
# First run: stores findings to "auth-audits" namespace
gaviero-cli --task "Document all authentication edge cases" \
  --namespace auth-audits

# Later run: reads prior findings, produces incremental update
gaviero-cli --task "Update authentication edge case docs with new OAuth flow" \
  --namespace auth-audits \
  --read-ns auth-audits
```

---

## Iteration flags

These override any `strategy` or `max_retries` declared in the `.gaviero` script:

```bash
# Retry up to 10 times per task
gaviero-cli --task "..." --max-retries 10

# Run 3 independent attempts and keep the best result
gaviero-cli --task "..." --attempts 3

# Generate failing tests first (TDD workflow)
gaviero-cli --task "..." --test-first --max-retries 5

# Disable retry loop (one shot, fast)
gaviero-cli --task "..." --no-iterate
```

---

## Multi-agent parallel execution

Set `--max-parallel > 1` to run independent agents concurrently. Each agent gets an isolated git worktree; branches are merged with automatic conflict resolution on completion.

```bash
gaviero-cli --script parallel_plan.gaviero --max-parallel 4
```

---

## Memory namespaces

Gaviero maintains per-project semantic memory. Agents read prior results to inform their work.

```bash
# Write to a specific namespace
gaviero-cli --task "Document the auth module" --namespace auth-docs

# Read from multiple namespaces
gaviero-cli --task "..." --namespace main --read-ns auth-docs --read-ns api-docs
```

---

## Output

### Text (default)

One line per agent on **stdout**. Status messages on **stderr**.

```
task-0: OK (src/auth.rs, tests/auth_test.rs)
```

### JSON (`--format json`)

```json
{
  "success": true,
  "manifests": [
    {
      "work_unit_id": "task-0",
      "status": "completed",
      "modified_files": ["src/auth.rs", "tests/auth_test.rs"],
      "summary": "Added validation to login endpoint",
      "cost_usd": 0.0023
    }
  ]
}
```

---

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All agents completed successfully |
| `1` | One or more agents failed or a merge conflict was not resolved |
