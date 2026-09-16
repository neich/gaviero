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
├─ main.rs          ~5.8 KLOC — Cli, observers, mode dispatch, helpers
└─ state.rs         workspace discovery, isolated state, agent-config restore
tests/
├─ remember_cli.rs     --remember integration tests
├─ history_cli.rs      --history integration tests
└─ state_modes_cli.rs  shared / --isolated / fallback runs against a fake Ollama
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

### Workspace state ([`src/state.rs`](src/state.rs))

The run root (`--repo` / `--workspace` / `PLAN_FILE` folder) is not necessarily where gaviero state lives. [`discover`](src/state.rs) walks the run root and its parent folders for the workspace the TUI would open. The marker is `.gaviero/settings.json` or `.gaviero/memory.db`; `$HOME` is skipped. A `*.gaviero-workspace` file listing that folder loads multi-folder mode. `Discovered` separates three roots:

| Root | Meaning |
|---|---|
| `marker_root` | folder where the search stopped |
| `state_root` | where the TUI keeps workspace state (its first folder): workspace memory DB, telemetry, history, graph |
| `memory_root` | workspace folder containing the run root; receives the run's repo-scoped memory |

`RunState::Shared` opens `MemoryStores::open(state_root, …)` with the configured embedder. It passes `SwarmConfig::memory_root` / `graph_db_path` and serves MCP at the run root with `GavieroMcpServer::with_memory_root` / `with_graph_db_path`. A run root below `state_root` gets its graph under `state_root/.gaviero/graphs/<hash>/`, because a graph build deletes every file it did not scan.

`RunState::Isolated` (`--isolated`, or nothing discovered) keeps memory, graph and telemetry in a `Scratch` temp dir (single store via `from_single_store`), serves MCP on an endpoint derived from that dir, disables skills, and wraps config synthesis in `ConfigRestore`. It refuses a run root with a live gaviero server. Memory/admin commands require a discovered workspace (`require_workspace`); `--history` / `--mcp-stats` fall back to the run root.

---

## Data Flow

```
parse Cli
  ├─ resolve run root; --mcp-register-user / --mcp-unregister-user exit here
  ├─ state::discover(run root)            (--isolated + admin flag → error)
  ├─ --mcp-stats / --history              (discovered state root, else run root)
  ├─ probe C1 migration on the discovered workspace (skipped for isolated runs)
  │
  ├─ one-shot admin modes (require a discovered workspace; exit before agents):
  │     --remember / --graph[--enrich]
  │     --manifest-* / --eval-* / --seed-corpus-from-paths
  │     --sleep / --utilization-* / --deletions-* / --restore-*
  │     --forget-* / --forget-history-id / --mcp-reach-probe
  ├─ --cleanup-branches (run root, git only)
  │
  ├─ RunState: Shared(discovered) | Isolated(scratch)
  ├─ open memory (best-effort; failure is non-fatal unless gaviero MCP is on)
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
  └─ else → pipeline::execute → [isolated: stop MCP, restore agent
            configs, remove scratch] → print SwarmResult → exit(0|1|2|3)
```

### Model resolution

[`resolve_model_spec`](src/main.rs) / [`backend_config_for_model`](../gaviero-core/src/swarm/backend/shared.rs):

```
claude:…  codex:…  cursor:…  ollama:…  local:…  deepseek:…  dsh:…
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
--auto-accept --resume --fresh --isolated --max-parallel
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
--history [--history-last N] [--history-conv ID] [--history-turn ID]
  [--history-json | --history-stats] [--history-path PATH]
--mcp-reach-probe [--reach-providers LIST] [--reach-depth N]
  [--reach-transport stdio|http|both] [--reach-json]
--mcp-register-user [claude,codex] --mcp-unregister-user [claude,codex]
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
