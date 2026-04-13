# gaviero-cli

Headless CLI runner for AI agent workflows. CI/automation friendly.

## Build & Test

```bash
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
cargo run -p gaviero-cli -- --help
```

Binary name: `gaviero-cli`

## Modes

| Flag | Description |
|---|---|
| `--task <text>` | Single task, full repo scope |
| `--work-units <json>` | JSON array of `WorkUnit` definitions |
| `--script <path>` | `.gaviero` DSL script file |
| `--coordinated` | Planner generates DAG from `--task`, outputs `.gaviero` for review |

## Key Flags

- `--model <spec>` — model string. Plain name (`sonnet`) / `claude:X` / `codex:X` / `ollama:X` / `local:X`. Default: sonnet.
- `--auto-accept` — skip interactive review
- `--resume` — resume interrupted run from checkpoint
- `--max-retries N` — inner-loop retries (iteration mode)
- `--attempts N` — independent attempts (BestOfN strategy)
- `--test-first` — TDD red phase: generate failing tests first
- `--format <text|json>` — output format
- `--namespace` / `--read-ns` — memory namespace control

## Structure

Single file: `src/main.rs`. Contains `Cli` struct (clap derive), `CliSwarmObserver`, `CliAcpObserver`.

## Dependencies

- `gaviero-core` — all pipeline logic
- `gaviero-dsl` — script compilation
- `clap` — argument parsing
- `miette` — error display
