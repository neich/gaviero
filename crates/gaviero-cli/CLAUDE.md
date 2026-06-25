# gaviero-cli

Headless CLI runner. Thin wrapper around `gaviero-core` + `gaviero-dsl` with stderr observers for agent / swarm / write-gate events.

Binary: `gaviero-cli` ([src/main.rs](src/main.rs), single source file).

## Build & Test

```bash
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

## Architecture

Single file: [`src/main.rs`](src/main.rs). Contains the `Cli` clap struct, mode dispatch, and two observers (`CliAcpObserver`, `CliSwarmObserver` — both write to stderr). Integration tests in [`tests/remember_cli.rs`](tests/remember_cli.rs).

The `Cli` struct is the **authoritative flag list** — read it before adding flag docs anywhere.

## Modes

| Flag | Mode |
|---|---|
| `--task <text>` | Single synthetic task, full-repo scope. |
| `--work-units <json>` | JSON array of `WorkUnit` objects. |
| `--script <path>` | `.gaviero` DSL file (use `--workflow <name>` to pick a workflow when multiple exist). |
| `--coordinated` | Planner emits a `.gaviero` DAG for review and exits. Pair with `--output`. |
| `--graph` | Build/update the repo-map knowledge graph and exit. |
| `--cleanup-branches` | Soft-list (or with `--force`, delete) leftover `gaviero/*` worktree branches. |
| `--resume` | Resume from `.gaviero/state/<plan-hash>.json`. |
| `--remember <text>` | Headless `/remember`-style memory write (`--remember-scope` selects: `run`/`module`/`repo`/`workspace`/`global`; default `repo`). |
| `--sleep` (+ `--sleep-dry-run`) | Run the sleeptime hygiene pass (B5) against the workspace `memory.db`. |
| `--utilization-scope <N>` (+ `--utilization-top`, `--utilization-asc`) | Top/least utilised memories at a scope (B6). |
| `--manifest-last <N>` / `--manifest-turn <id>` | Print retrieval manifests (S4) and exit. |
| `--deletions-last <N>` / `--restore-id <id>` / `--restore-since <when>` | Soft-delete audit (C2.2). |
| `--forget-query` / `--forget-scope` / `--forget-type` / `--forget-source` (+ `--forget-dry-run`, `--forget-yes`, `--forget-reason`) | Bulk soft-delete (C2.3). |
| `--forget-history-id <id>` (+ `--redact-confirm <literal>`, `--redact-reason <text>`) | History row redaction (C2.4). |
| `--eval-fixture <path>` (+ `--eval-tolerance`, `--eval-update-baseline`, `--eval-report-out`, `--eval-allow-missing-baseline`, `--eval-rerank-ablation`, `--eval-embedder-ablation`, `--eval-budget-sweep`, `--eval-from-manifests <N>`, `--eval-bootstrap-from-manifests <N>`, `--eval-scope-matrix`, `--eval-scope-matrix-scopes <list>`, `--seed-corpus-from-paths`) | Memory recall@K / MRR regression harness. |
| `--seed-corpus-from-paths` (+ `--seed-corpus-doc-chars`) | T2 corpus seeding from `gold_must` File entries. |
| `--accept-c1-migration` | Accept the typed-stores schema migration on first launch (C1). |

## Key Flags (cross-mode)

- `--model <spec>` — `claude:X` / `codex:X` / `cursor:X` / `ollama:X` / `local:X`. `provider:` prefix is required; bare names are rejected. Default: workspace `agent.model`, then `claude:sonnet`.
- `--coordinator-model <spec>` — planner model in `--coordinated` mode.
- `--ollama-base-url <url>` — override Ollama endpoint.
- `--workflow <name>` — pick a workflow when the DSL script defines several.
- `--auto-accept` — skip interactive review (all writes pass through the Write Gate in `AutoAccept` mode).
- `--max-retries N` / `--attempts N` / `--no-iterate` / `--test-first` — iteration controls.
- `--namespace <name>` / `--read-ns <name>` (repeatable) — memory scope control.
- `--var KEY=VALUE` (repeatable, `--script` only) — override script-level `vars {}`.
- `--param NAME=VALUE` (repeatable, `--script` only) — supply a workflow-level `param <name>` declaration. Roster params: `id=provider:model[@effort],...`. Client params: `provider:model[@effort]`. Required params (no in-script default) fail compilation when absent. See [`gaviero-dsl::workflow_params`](../gaviero-dsl/src/workflow_params.rs).
- `--tiers-file <path>` (`--script` only) — tier profile with only `tier <alias> <client-ref>` lines; overrides bindings from the script and any `include`d files. See [`gaviero-dsl::tiers`](../gaviero-dsl/src/tiers.rs).
- `--prompt-file <path>` (`--script` only) — file contents replace every `{{PROMPT}}` and become the default prompt for agents with no `prompt` field.
- `--trace <path>` — structured JSON trace; enables DEBUG-level tracing.
- `--verbose` / `-v` (repeatable, max `-vv`) — stderr log level (INFO → DEBUG).
- `--format <text|json>` — output format on stdout (`json` for machine consumption).
- `--exclude <name|glob>` (repeatable, comma-separated) — skip paths during repo-map scan.
- `--repo <path>` — workspace root (default: `.`).

## Conventions

- **stdout = results, stderr = telemetry.** Observers always log to stderr; only the final mode-specific output (plan path, JSON result, etc.) is on stdout.
- **Model spec follows the workspace rule.** Provider prefix mandatory; `validate_model_spec` ([`../gaviero-core/src/swarm/backend/shared.rs`](../gaviero-core/src/swarm/backend/shared.rs)) is authoritative.
- **DSL precedence.** Tier bindings: `--tiers-file` > script/includes `tier` lines. Var precedence: agent-level `vars {}` > `--var` overrides > script-level `vars {}` (see [`gaviero-dsl`](../gaviero-dsl/CLAUDE.md)).
- **Memory operations open the same `MemoryServices`** as the TUI; never bypass the writer task.
- **Exit codes:** `0` success, non-zero on compile/validation/runtime error; structured failures still print a `MietteReport` to stderr.

## Rules

- **Never add a flag here without adding the field on the `Cli` struct first.** The struct is the source of truth — docs (this file, README.md, ARCHITECTURE.md) trail it.
- **Never write directly to a file.** Even synthetic-task mode goes through the Write Gate (`AutoAccept` when `--auto-accept` is set).
- **Tier overrides only contain `tier` lines.** [`load_tier_overrides`](../gaviero-dsl/src/tiers.rs) rejects other items — keep that surface narrow.
- **Always validate the model spec early.** Defer to `gaviero-core` rather than parsing prefixes in CLI code.

## Dependencies

- `gaviero-core`, `gaviero-dsl` — pipeline + DSL compilation.
- `clap 4` (derive) — flag parsing.
- `miette` — diagnostics from `gaviero-dsl`.
- `tokio`, `tracing`, `tracing-subscriber`, `serde_json`, `anyhow`, `dirs`.

## See Also

- [README.md](README.md) — examples and flag reference.
- [ARCHITECTURE.md](ARCHITECTURE.md) — coordinated mode, memory integration, exit-code table, observer wiring.
- [`gaviero-dsl/CLAUDE.md`](../gaviero-dsl/CLAUDE.md) — script semantics, var/tier precedence.
