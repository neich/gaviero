# gaviero-cli

Headless CLI runner. Thin wrapper around `gaviero-core` + `gaviero-dsl`.

Binary: `gaviero-cli`

## Build & Test

```bash
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

## Modes

- `--task <text>` — single task, full repo scope
- `--work-units <json>` — JSON array of `WorkUnit` objects
- `--script <path>` — `.gaviero` DSL file
- `--coordinated` — planner generates DAG from task, emits `.gaviero` for review and exits

## Key Flags

- `--model <spec>` — `sonnet` / `opus` / `haiku` / `claude:X` / `codex:X` / `ollama:X` / `local:X` (defaults to workspace `agent.model`, then `sonnet`)
- `--coordinator-model <spec>` — model used for coordinated planning
- `--ollama-base-url <url>` — override Ollama endpoint
- `--auto-accept` — skip interactive review
- `--resume` — resume from `.gaviero/state/<plan-hash>.json`
- `--max-retries N` — inner-loop retries
- `--attempts N` — independent attempts (BestOfN)
- `--test-first` — TDD red phase
- `--no-iterate` — single pass only
- `--format <text|json>` — output format
- `--namespace <name>` / `--read-ns <name>` — memory scope control (repeatable for `--read-ns`)
- `--var KEY=VALUE` — override a script-level `vars {}` entry (repeatable; `--script` only)
- `--prompt-file <path>` — file contents replace every `{{PROMPT}}` in the script and become the default prompt for agents with no `prompt` field (`--script` only)
- `--trace-log <path>` — structured JSON trace
- `--plan-output <path>` — output `.gaviero` path (`--coordinated` only)
- `--graph` — build/update the repo-map knowledge graph and exit
- `--exclude <name|glob>` — skip paths during graph scan (repeatable, comma-separated)

## Structure

Single file: `src/main.rs`. Contains `Cli` struct (clap) and the stderr observers for agent / swarm / write-gate events.

## Dependencies

- `gaviero-core`, `gaviero-dsl`
- `clap`, `miette`, `tracing`

## See Also

[README.md](README.md) — flags reference, examples, use cases.
