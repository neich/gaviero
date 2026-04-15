# gaviero-cli

Headless CLI runner. Thin wrapper around gaviero-core.

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
- `--coordinated` — planner generates DAG from task, outputs .gaviero for review

## Key Flags

- `--model <spec>` — model string: `sonnet` / `claude:X` / `codex:X` / `ollama:X` / `local:X` (default: sonnet)
- `--auto-accept` — skip interactive review
- `--resume` — resume from checkpoint
- `--max-retries N` — inner-loop retries
- `--attempts N` — independent attempts (BestOfN)
- `--test-first` — TDD red phase
- `--format <text|json>` — output format
- `--namespace` / `--read-ns` — memory scope control

## Structure

Single file: `src/main.rs`. Contains `Cli` struct (clap), observers.

## Dependencies

- `gaviero-core`, `gaviero-dsl`
- `clap`, `miette`

## See Also

[README.md](README.md) — flags reference, examples, use cases.
