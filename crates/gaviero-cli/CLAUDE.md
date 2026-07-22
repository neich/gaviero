# gaviero-cli

Headless CLI runner. Thin wrapper around `gaviero-core` + `gaviero-dsl` with stderr observers for agent / swarm / write-gate events.

Binary: `gaviero-cli` ([src/main.rs](src/main.rs), single ~4 KLOC source file).

## Build & Test

```bash
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

Integration tests: [`tests/remember_cli.rs`](tests/remember_cli.rs). Live eval example: [`examples/anchor_ab_live.rs`](examples/anchor_ab_live.rs).

## Architecture

[`src/main.rs`](src/main.rs) owns the `Cli` clap struct, mode dispatch, and two observers (`CliAcpObserver`, `CliSwarmObserver` — stderr only).

**`Cli` is the authoritative flag list.** Read the struct before documenting or inventing flags. There is **no** `--no-memory` flag.

Mode families (flags on `Cli`):

| Family | Entry flags |
|---|---|
| Swarm / script | `--task`, `--work-units`, `--script`, `--coordinated`, `--resume` |
| Graph | `--graph` (+ `--enrich` / `--enrich-no-embed`) |
| Git hygiene | `--cleanup-branches` (+ `--force`) |
| Memory admin | `--remember`, `--sleep`, `--utilization-scope`, `--manifest-*`, `--deletions-*` / `--restore-*`, `--forget-*` |
| MCP | `--mcp-stats` (+ `--mcp-stats-path`); runtime: `--no-mcp`, `--mcp-url`, `--mcp-stdio`, `--mcp-codex-trust`, `--skip-mcp-preflight` |
| Eval | `--eval-fixture` (+ ablation / budget / anchor-ab / scope-matrix / seed-corpus flags) |

Full user-facing flag tables: [README.md](README.md). Do not duplicate every field here.

## Conventions

- **stdout = results, stderr = telemetry.** Observers always log to stderr.
- **Model spec:** `provider:model` required. Accepted prefixes include `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:` ([`validate_model_spec`](../gaviero-core/src/swarm/backend/shared.rs)). Default: workspace `agent.model`, then `claude:sonnet`.
- **`--repo` vs `--workspace`:** `execution repo` vs `execution document` ([`workflow_execution_mode`](../gaviero-dsl/src/lib.rs)). Conflicts with each other; `--workspace` defaults to the plan file's directory when `--var PLAN_FILE=...` is set.
- **DSL precedence.** Tiers: `--tiers-file` > script/includes. Vars: agent-level > `--var` > script-level. Params: `--param` (see [`workflow_params`](../gaviero-dsl/src/workflow_params.rs)).
- **Memory ops open the same `MemoryServices`** as the TUI; never bypass the writer task.
- **Exit codes:** `0` success; non-zero on compile/validation/runtime error; structured failures print `MietteReport` to stderr.

## Rules

- **Never add a flag without adding the field on `Cli` first.** Docs trail the struct.
- **Never write directly to a file.** Synthetic-task mode still goes through the Write Gate (`AutoAccept` when `--auto-accept`).
- **Tier overrides only contain `tier` lines.** [`load_tier_overrides`](../gaviero-dsl/src/tiers.rs) rejects other items.
- **Always validate the model spec via core** — do not re-parse prefixes in CLI code.
- **Never document `--no-memory`.** It does not exist on `Cli`.

## Dependencies

- `gaviero-core`, `gaviero-dsl` — pipeline + DSL compilation.
- `clap 4` (derive) — flag parsing.
- `miette` — diagnostics from `gaviero-dsl`.
- `tokio`, `tracing`, `tracing-subscriber`, `serde_json`, `anyhow`, `dirs`.

## See Also

- [README.md](README.md) — examples and complete flag reference.
- [ARCHITECTURE.md](ARCHITECTURE.md) — coordinated mode, memory integration, exit codes, observer wiring.
- [`../gaviero-dsl/CLAUDE.md`](../gaviero-dsl/CLAUDE.md) — script semantics, var/tier/param precedence.
