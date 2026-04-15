# gaviero-cli

Headless CLI runner for AI agent tasks. Execute single tasks, run DSL workflows, or generate coordinated plans from the command line or CI pipelines. All logic delegates to `gaviero-core`.

## Installation & Build

```bash
cargo build -p gaviero-cli
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

Binary name: `gaviero-cli`

## Overview

Gaviero CLI provides three ways to define work:

1. **Single task** — Simple task description with auto-generated scope
2. **Workflow script** — Compiled `.gaviero` DSL file with agents and verification
3. **Work units JSON** — Explicit structured task definitions
4. **Coordinated planning** — Auto-decompose a task into a reviewable `.gaviero` plan

Execution is non-interactive. Results and progress go to stdout/stderr.

## Usage: Single Task

Create one agent with full repository scope:

```bash
gaviero-cli --task "Fix compilation errors in the auth module"
gaviero-cli --repo ~/my-project --task "Refactor database layer"
```

## Usage: DSL Workflow

Execute a pre-written `.gaviero` workflow file:

```bash
gaviero-cli --script workflows/review_and_fix.gaviero
gaviero-cli --repo ~/my-project --script ./ci/refactor.gaviero
```

Model strings in the `.gaviero` file are respected. CLI `--model` sets the default if the file doesn't specify.

## Usage: Work Units (JSON)

Pass explicit work definitions as JSON:

```bash
gaviero-cli --work-units '[
  {
    "id": "design",
    "description": "Plan the refactor",
    "scope": {"owned_paths": ["src/"], "read_only_paths": ["docs/"]}
  },
  {
    "id": "implement",
    "description": "Apply the plan",
    "depends_on": ["design"],
    "scope": {"owned_paths": ["src/"]}
  }
]'
```

## Usage: Coordinated Planning

Generate a `.gaviero` plan without executing:

```bash
gaviero-cli --coordinated \
  --task "Split billing into planning, execution, and verification layers"
```

The generated plan is printed to stdout or saved via `--output`:

```bash
gaviero-cli --coordinated \
  --task "Refactor auth module" \
  --output tmp/plan.gaviero
```

Then review and execute:

```bash
gaviero-cli --script tmp/plan.gaviero
```

## Model Routing

The CLI supports provider-aware model specifications:

### Model spec formats

- **Claude** — `sonnet`, `opus`, `haiku` or `claude:sonnet`, `claude-code:haiku`
- **Ollama/local** — `ollama:qwen2.5-coder:7b` or `local:model-name`

### Priority resolution

```
1. --model flag (if provided)
2. workspace agent.model setting
3. default: sonnet
```

For coordinated planning, override the coordinator model:

```bash
gaviero-cli --coordinated \
  --task "..." \
  --model ollama:qwen2.5-coder:7b \
  --coordinator-model claude:sonnet
```

Ollama server URL precedence:

```
1. --ollama-base-url flag
2. workspace agent.ollamaBaseUrl
3. default: http://localhost:11434
```

## Flag Reference

| Flag | Argument | Purpose |
|---|---|---|
| `--repo` | `<path>` | Workspace root (default: current directory) |
| `--task` | `<text>` | Single-task mode — auto-scoped, full repo |
| `--script` | `<path>` | `.gaviero` DSL workflow file |
| `--work-units` | `<json>` | Explicit work unit definitions |
| `--coordinated` | — | Generate reviewable plan, don't execute |
| `--output` | `<path>` | Save generated plan to file (`--coordinated`) |
| `--model` | `<spec>` | Execution model (default: sonnet) |
| `--coordinator-model` | `<spec>` | Planner model for `--coordinated` |
| `--ollama-base-url` | `<url>` | Ollama server URL |
| `--auto-accept` | — | Skip interactive review, apply changes directly |
| `--max-parallel` | `<n>` | Override workflow parallelism |
| `--max-retries` | `<n>` | Retry limit for validation feedback |
| `--attempts` | `<n>` | Independent attempts (best-of-N strategy) |
| `--test-first` | — | Generate failing tests before code changes |
| `--no-iterate` | — | Single-pass execution, no retries |
| `--resume` | — | Resume from saved checkpoint |
| `--namespace` | `<ns>` | Memory write namespace |
| `--read-ns` | `<ns>` | Additional read namespaces (repeatable) |
| `--format` | `text\|json` | Output format |
| `--trace` | `<file>` | Write DEBUG-level JSON trace log |
| `--graph` | — | Build code graph and exit |

## Output

**Standard output** — Results, plan files, structured data (JSON mode)

**Standard error** — Progress, observer events, logs

This split allows shell pipelines to capture results without losing telemetry:

```bash
gaviero-cli --task "..." --format json > results.json 2> progress.log
```

## Examples

### Simple task, auto-accept

```bash
gaviero-cli --repo ~/my-project \
  --task "Add comprehensive error handling to API routes" \
  --auto-accept
```

### Multi-agent workflow with test-first

```bash
gaviero-cli \
  --script workflows/feature-branch.gaviero \
  --test-first \
  --max-retries 3
```

### Use local Ollama for cost-sensitive work

```bash
gaviero-cli \
  --task "Update docstrings" \
  --model ollama:qwen2.5-coder:7b \
  --ollama-base-url http://localhost:11434
```

### Best-of-3 execution

```bash
gaviero-cli \
  --task "Refactor database schema" \
  --attempts 3 \
  --format json > results.json
```

## See Also

- [Root README](../../README.md) — full feature overview
- [crates/gaviero-core/README.md](../gaviero-core/README.md) — execution engine
- [crates/gaviero-dsl/README.md](../gaviero-dsl/README.md) — workflow language
