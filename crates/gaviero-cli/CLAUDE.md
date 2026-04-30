# gaviero-cli

Headless CLI runner. Thin wrapper around `gaviero-core` + `gaviero-dsl` with stderr observers for agent / swarm / write-gate events.

Binary: `gaviero-cli`

## Build & Test

```bash
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

## Modes

- `--task <text>` — single task, full repo scope
- `--work-units <json>` — JSON array of `WorkUnit` objects
- `--script <path>` — `.gaviero` DSL file (use `--workflow <name>` to pick a workflow)
- `--coordinated` — planner generates DAG from task, emits `.gaviero` for review and exits

## Key Flags

- `--model <spec>` — `sonnet` / `opus` / `haiku` / `claude:X` / `codex:X` / `ollama:X` / `local:X` (defaults to workspace `agent.model`, then `sonnet`)
- `--coordinator-model <spec>` — model used for coordinated planning
- `--ollama-base-url <url>` — override Ollama endpoint
- `--workflow <name>` — pick a workflow when the script defines several
- `--auto-accept` — skip interactive review
- `--resume` — resume from `.gaviero/state/<plan-hash>.json`
- `--max-retries N` — inner-loop retries
- `--attempts N` — independent attempts (BestOfN)
- `--test-first` — TDD red phase
- `--no-iterate` — single pass only
- `--format <text|json>` — output format
- `--namespace <name>` / `--read-ns <name>` — memory scope control (`--read-ns` repeatable)
- `--no-memory` — disable memory subsystem for this run
- `--var KEY=VALUE` — override script-level `vars {}` (repeatable; `--script` only)
- `--prompt-file <path>` — file contents replace every `{{PROMPT}}` and become the default prompt for agents with no `prompt` field (`--script` only)
- `--trace-log <path>` — structured JSON trace
- `--plan-output <path>` — output `.gaviero` path (`--coordinated` only)
- `--graph` — build/update the repo-map knowledge graph and exit
- `--exclude <name|glob>` — skip paths during graph scan (repeatable, comma-separated)

## Structure

Single source file: `src/main.rs` (~2.1 KLOC). Contains the `Cli` clap struct, mode dispatch, and stderr observers. Tests in `tests/remember_cli.rs`.

The authoritative flag list is the `Cli` struct — read it before adding flags here.

## Dependencies

- `gaviero-core`, `gaviero-dsl`
- `clap`, `miette`, `tracing`

## See Also

- [README.md](README.md) — flags reference, examples, use cases
- [ARCHITECTURE.md](ARCHITECTURE.md) — coordinated mode, memory integration, full flag reference
