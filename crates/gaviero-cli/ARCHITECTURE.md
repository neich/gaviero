# gaviero-cli — Architecture

Headless runner. Clap front end + stderr observers; all runtime work delegates to [`gaviero-core`](../gaviero-core) and [`gaviero-dsl`](../gaviero-dsl).

Binary: `gaviero-cli`. Conventions: [CLAUDE.md](CLAUDE.md). Flag examples: [README.md](README.md).

---

## Topology

```
gaviero-cli (binary)
    │ parse Cli (clap) → pick one mode → wire stderr observers
    ▼
gaviero-dsl::compile_file / compile_with_vars     (--script)
    │ CompiledPlan
    ▼
gaviero-core::swarm::pipeline::execute
             ::swarm::coordinator::plan_coordinated
             ::memory::* / ::repo_map::* / ::mcp::*
    │
    ▼ stdout (results) + stderr (observers)
```

Intentionally thin: parse flags, select a mode, wire [`CliAcpObserver`](src/main.rs) / [`CliSwarmObserver`](src/main.rs), delegate. No business logic beyond mode dispatch and path/workspace prep helpers in the same file.

---

## Modules

```
gaviero-cli/src/
└─ main.rs          ~4000 lines — Cli, observers, mode dispatch, helpers
tests/
└─ remember_cli.rs  --remember integration tests
examples/
└─ anchor_ab_live.rs  live A/B harness for eval-anchor-ab
```

The [`Cli`](src/main.rs) struct is the **authoritative** flag list. Do not document flags that are not fields on `Cli` (there is **no** `--no-memory`).

---

## Abstractions

### `Cli` ([`src/main.rs`](src/main.rs))

Single clap-derived struct covering every operating mode: swarm execution, coordinated planning, graph/enrich, memory admin, MCP stats, eval harness family.

### Observers

- [`CliAcpObserver`](src/main.rs) — stream / tools / validation / deferred proposals / token usage → stderr (`[{agent_id}]` prefix).
- [`CliSwarmObserver`](src/main.rs) — phase / agent / tier / merge / cost / completion → stderr.

Stdout stays clean for `--format json`.

### Workspace prep

Helpers (`prepare_swarm_workspace`, `materialize_external_vars_for_repo`, `mcp_overrides_from_cli`, …) anchor `--repo` / `--workspace`, copy external var files into worktrees, and synthesize MCP overrides before `pipeline::execute`.

---

## Data Flow

```
parse Cli
  ├─ open Workspace + settings
  ├─ probe C1 migration (refuse without --accept-c1-migration)
  ├─ init MemoryStores (best-effort; failure is non-fatal)
  │
  ├─ one-shot admin modes (exit before agents):
  │     --remember / --graph[--enrich] / --cleanup-branches
  │     --manifest-* / --eval-* / --seed-corpus-from-paths
  │     --sleep / --utilization-* / --deletions-* / --restore-*
  │     --forget-* / --forget-history-id / --mcp-stats
  │
  ├─ plan input:
  │     --task            → synthetic WorkUnit (owned=["."])
  │     --work-units      → Vec<WorkUnit> JSON
  │     --script          → gaviero_dsl::compile_file(..., override_vars,
  │                          override_tiers, override_params)
  │                        + --workflow / --prompt / --prompt-file
  │                        + --var / --param / --tiers-file
  │
  ├─ iteration overlays (--max-retries, --attempts, --test-first, --no-iterate)
  ├─ --coordinated? → plan_coordinated → write .gaviero → exit
  └─ else → pipeline::execute → print SwarmResult → exit(0|1|2|3)
```

### Model resolution

[`resolve_model_spec`](src/main.rs) / [`backend_config_for_model`](../gaviero-core/src/swarm/backend/shared.rs):

```
claude:…  codex:…  cursor:…  ollama:…  local:…  deepseek:…
```

Bare names rejected. `--coordinator-model` for `--coordinated`. `--ollama-base-url` overrides Ollama endpoint.

---

## Concurrency

Tokio runtime in `main`. Observers are sync stderr writers; swarm / memory work runs on the shared runtime. No CLI-local locks. Memory writes go through core's single writer task.

---

## Error Handling

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | agent / validation / merge / eval regression / abort |
| 2 | argument error |
| 3 | setup (workspace / memory / panic / pending C1) |

DSL errors print as `miette::Report` with spans. Fatal paths use `anyhow::Context`. Pending C1 migration prints affected DB paths + backup proposal.

---

## API

Binary only — no library API. Public surface is the CLI:

### Input / execution (see `Cli`)

```
--repo / --workspace
--task | --work-units | --script
--workflow --prompt --prompt-file
--var KEY=VALUE --param NAME=VALUE --tiers-file
--model --coordinator-model --ollama-base-url
--auto-accept --resume --max-parallel
--max-retries --attempts --test-first --no-iterate
--coordinated --output
--format text|json --trace --verbose/-v
```

### Graph / MCP / memory admin

```
--graph [--enrich [--enrich-no-embed]] --exclude
--cleanup-branches [--force]
--no-mcp --mcp-url --mcp-stdio --mcp-codex-trust
--skip-mcp-preflight --mcp-stats [--mcp-stats-path]
--namespace --read-ns --accept-c1-migration
--remember [--remember-scope]
--manifest-last / --manifest-turn
--sleep [--sleep-dry-run] --utilization-*
--deletions-last --restore-id --restore-since
--forget-query|scope|type|source [--forget-dry-run|--forget-yes] [--forget-reason]
--forget-history-id --redact-confirm --redact-reason
```

### Eval family

```
--eval-fixture [--eval-tolerance] [--eval-report-out]
  [--eval-update-baseline] [--eval-allow-missing-baseline]
  [--eval-rerank-ablation] [--eval-embedder-ablation]
  [--eval-budget-sweep] [--eval-anchor-ab]
--eval-from-manifests --eval-bootstrap-from-manifests
--eval-scope-matrix [--eval-scope-matrix-scopes]
--seed-corpus-from-paths [--seed-corpus-doc-chars]
```

Re-read [`Cli`](src/main.rs) before adding flag docs. Dependencies: `gaviero-core`, `gaviero-dsl`, clap, tokio, serde_json, miette, anyhow, tracing.
