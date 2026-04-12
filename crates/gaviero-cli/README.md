# gaviero-cli

`gaviero-cli` is the headless runner for Gaviero. It is the simplest way to
execute a single task, run a compiled DSL workflow, or generate a reviewable
coordinated plan from the command line.

## Input modes

Exactly one input mode is required.

### 1. `--task`

Create one synthetic work unit with broad repository scope.

```bash
gaviero-cli --task "Fix the panic in the auth session loader"
```

### 2. `--script`

Compile and execute a `.gaviero` workflow.

```bash
gaviero-cli --script workflows/refactor_auth.gaviero
```

### 3. `--work-units`

Pass serialized `WorkUnit` JSON directly.

```bash
gaviero-cli --work-units '[{"id":"t1","description":"...","scope":{"owned_paths":["src/"],"read_only_paths":[],"interface_contracts":{}}}]'
```

## Coordinated planning

Use `--coordinated` with `--task` to generate a reviewable `.gaviero` plan and
exit without running any agents.

```bash
gaviero-cli --coordinated \
  --task "Split the billing code into planning, execution, and verification steps"
```

The generated file can then be reviewed and executed:

```bash
gaviero-cli --script tmp/gaviero_plan_<timestamp>.gaviero
```

## Model behavior

The CLI now accepts provider-aware model specs directly.

- `--model <spec>` sets the synthetic task model and the default runtime model
- `--coordinator-model <spec>` overrides the planner model for `--coordinated`
- `--script` still respects explicit model strings inside the `.gaviero` file

Accepted forms:

- `sonnet`, `opus`, `haiku`
- `claude:sonnet`, `claude-code:haiku`
- `ollama:qwen2.5-coder:7b`, `local:qwen2.5-coder:14b`

Default precedence:

- execution model: `--model` > workspace `agent.model` > `sonnet`
- coordinator model: `--coordinator-model` > workspace `agent.coordinator.model` > execution model
- Ollama URL: `--ollama-base-url` > workspace `agent.ollamaBaseUrl` > backend default

Examples:

```bash
gaviero-cli --task "Refactor the auth module" \
  --model ollama:qwen2.5-coder:7b

gaviero-cli --coordinated \
  --task "Split billing into planner, executor, and verifier" \
  --model ollama:qwen2.5-coder:7b \
  --coordinator-model claude:sonnet
```

## Useful flags

| Flag | Purpose |
| --- | --- |
| `--repo <path>` | Open a specific workspace root |
| `--model <spec>` | Execution/default model spec |
| `--coordinator-model <spec>` | Planner model spec for `--coordinated` |
| `--ollama-base-url <url>` | Override Ollama server URL |
| `--auto-accept` | Skip interactive file review and apply writes directly |
| `--max-parallel <n>` | Override workflow-level parallelism when applicable |
| `--max-retries <n>` | Override inner validation retry count |
| `--attempts <n>` | Enable best-of-N style repeated attempts |
| `--test-first` | Ask the runtime to generate failing tests before editing |
| `--no-iterate` | Force single-pass execution |
| `--resume` | Resume from stored execution state |
| `--format text|json` | Choose human-readable or machine-readable result output |
| `--trace <file>` | Write DEBUG-level JSON tracing |
| `--graph` | Build/update the code graph and exit |

## Output behavior

- Results go to `stdout`
- Progress and observer events go to `stderr`

That split is deliberate so shell pipelines can safely capture structured output
without losing execution telemetry.

## When to use the CLI

Use `gaviero-cli` when you want:

- CI or scripted execution
- a non-interactive runner
- to generate a coordinated plan from the shell
- to execute a checked-in `.gaviero` workflow without the TUI

Use `gaviero-tui` when you want interactive review, editor-driven planning, or
chat-first workflows.
