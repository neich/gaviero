# gaviero-cli — Architecture

Headless runner. [`clap`](https://docs.rs/clap) front end + observer wiring; all runtime work delegates to [`gaviero-core`](../gaviero-core) and [`gaviero-dsl`](../gaviero-dsl).

Binary: `gaviero-cli`

---

## 1. Topology

```
                    ┌──────────────────────────────┐
                    │     gaviero-cli (binary)     │
                    └──────────┬───────────────────┘
                               │ parse Cli (clap)
                               │ pick mode
                               │ wire stderr observers
                               ▼
            ┌────────────────────────────────────────────┐
            │           gaviero-dsl::compile_*           │  (--script paths)
            └────────────────────┬───────────────────────┘
                                 │ CompiledPlan
                                 ▼
            ┌────────────────────────────────────────────┐
            │     gaviero-core::swarm::pipeline::execute │
            │     gaviero-core::swarm::coordinator       │
            │     gaviero-core::memory::*                │
            └────────────────────────────────────────────┘
                                 │
                                 ▼ stdout (results) + stderr (observers)
```

Intentionally minimal:
- Parse flags with `clap` (single `Cli` struct, derive macro).
- Pick exactly one operating mode — execution, coordinated planning, repo-map build, eval, sleeptime, memory administration, `/remember`, or `--cleanup-branches`.
- Wire stderr observers ([`AcpObserver`](../gaviero-core/src/observer.rs), [`SwarmObserver`](../gaviero-core/src/observer.rs), write-gate logging).
- Delegate to `gaviero_core` / `gaviero_dsl`.
- Write results to stdout (text or JSON) unless `--output <path>` is set.

---

## 2. Module Map

```
gaviero-cli/src/
└─ main.rs    2586 lines. Contains:
                Cli            — clap-derived flag list (authoritative)
                OutputFormat   — Text | Json
                CliAcpObserver — stream / tool / validation / retries → stderr
                CliSwarmObserver — phase / agent / cost → stderr
                run()          — mode dispatch
                main()         — runtime + panic boundary
tests/
└─ remember_cli.rs   integration tests for --remember flow
```

The `Cli` struct is the authoritative flag list — read it before adding flags here.

---

## 3. Execution Flow

```
parse Cli (clap)
   │
   ├─ open Workspace + settings (gaviero_core::workspace)
   ├─ probe C1 migration; refuse without --accept-c1-migration
   ├─ init MemoryStores (best-effort; --no-memory bypasses)
   │
   ├─ admin / one-shot modes (each exits before agent dispatch):
   │     --remember <text>                 → WriterHandle store, exit
   │     --graph                           → repo-map build/refresh, stats, exit
   │     --cleanup-branches [+ --force]    → list/delete gaviero/* branches, exit
   │     --manifest-last / --manifest-turn → print injection_manifests, exit
   │     --eval-fixture / --eval-from-… /  → run eval harness, write report, exit
   │       --eval-bootstrap-from-manifests
   │       --eval-scope-matrix
   │     --seed-corpus-from-paths          → bootstrap T2 corpus, exit
   │     --sleep [+ --sleep-dry-run]       → run_sleeptime, exit
   │     --utilization-scope               → print top/bottom-N utilization, exit
   │     --deletions-last                  → list audit rows, exit
   │     --restore-id / --restore-since    → replay deletion through dedup, exit
   │     --forget-* (+ --forget-yes)       → soft-delete via writer task, exit
   │     --forget-history-id (REDACT)      → C2.4 in-place redaction, exit
   │
   ├─ task input mode:
   │     --task <text>          → synthetic WorkUnit (full repo scope)
   │     --work-units <json>    → Vec<WorkUnit> (serde)
   │     --script <path>        → gaviero_dsl::compile_file(...)
   │                              (resolver honours `include` transitively;
   │                              --tiers-file + --var threaded through)
   │
   ├─ apply iteration overrides (--max-retries, --attempts,
   │                             --test-first, --escalate-after, --no-iterate)
   │
   ├─ --coordinated?
   │     yes → swarm::coordinator::plan_coordinated → write .gaviero, exit
   │     no  → swarm::pipeline::execute
   │
   └─ print SwarmResult (text or JSON) → exit(0/1/2/3)
```

---

## 4. Input Modes

### `--task <text>`

Creates a synthetic `WorkUnit` with scope `owned_paths = ["."]`, `tier = Cheap`, `privacy = Public`, model from `--model` or workspace default. Wrapped in a single-node `CompiledPlan::from_work_units(vec![unit])`.

### `--work-units <json>`

`serde_json::from_str::<Vec<WorkUnit>>` then `CompiledPlan::from_work_units`.

### `--script <path>`

```rust
let plan = gaviero_dsl::compile_file(
    path,
    workflow,
    runtime_prompt,
    &override_vars,
    &override_tiers,
)?;
```

- `--workflow <name>` selects a workflow when the script declares multiple.
- `--prompt-file <path>` reads file contents and supplies them as `runtime_prompt` (replaces every `{{PROMPT}}`; also acts as the full prompt for agents without a `prompt` field).
- `--var KEY=VALUE` (repeatable) overrides entries in the script's top-level `vars {}`. Precedence: agent-level `vars {}` > CLI `--var` > script-level `vars {}`.
- `--tiers-file <profile.gaviero>` loads `tier <alias> <client>` bindings via [`gaviero_dsl::load_tier_overrides`](../gaviero-dsl/src/tiers.rs); overrides script + `include` tier lines.

`compile_file` runs the include resolver first, so `--script` works with multi-file workflows (`include "..."`) transparently.

---

## 5. Observers

### `CliAcpObserver`

Prints stream chunks (verbose only), tool calls, validation results, retries, and deferred proposals to stderr. Lines are prefixed `[{agent_id}]` for multi-agent tracing. Implements every method the agent surfaces — including `on_cursor_session_started` (logs the Cursor thread id) and `on_turn_token_usage`.

### `CliSwarmObserver`

Prints phase, agent state, tier start/dispatch, merge conflicts, cost updates, and completion to stderr.

All observer events route to stderr so stdout stays clean for structured output (`--format json`).

---

## 6. Model Resolution

Model spec passed via `--model` (or the DSL `client` decl) is parsed by [`gaviero_core::swarm::backend::shared::backend_config_for_model`](../gaviero-core/src/swarm/backend/shared.rs). Specs are required to be in canonical `provider:model` form — `validate_model_spec` rejects bare names without a provider prefix.

```
claude:<name>               → Claude API (ACP)
codex:<name>                → Codex exec
cursor:<name>               → Cursor CLI (NDJSON stream-json)
ollama:<name>               → Ollama HTTP SSE
local:<name>                → Ollama HTTP SSE (alias)
```

`--coordinator-model` picks the planner model for `--coordinated`. `--ollama-base-url` overrides the Ollama endpoint (defaults to workspace setting, then `http://localhost:11434`).

[`TierRouter`](../gaviero-core/src/swarm/router.rs) maps `(ModelTier, PrivacyLevel)` to a concrete backend; [`PrivacyScanner`](../gaviero-core/src/swarm/privacy.rs) can promote a unit to `LocalOnly` based on glob matches against sensitive paths.

---

## 7. Iteration Overrides

| Flag | Effect on `IterationConfig` |
|---|---|
| `--max-retries N` | inner-loop retries per attempt (default 5) |
| `--attempts N` | independent attempts → `Strategy::BestOfN(N)` |
| `--test-first` | TDD red phase before the edit loop |
| `--escalate-after M` | escalate tier after M failed attempts |
| `--no-iterate` | single pass (overrides `--max-retries`) |

Applied after the `CompiledPlan` is produced, before `pipeline::execute`.

---

## 8. Coordinated Mode

```
gaviero-cli --task "..." --coordinated --model claude:opus
```

Calls [`swarm::coordinator::plan_coordinated`](../gaviero-core/src/swarm/coordinator.rs), which emits a `CompiledPlan`. The plan is serialised back to `.gaviero` form and written to `--output` (or `tmp/gaviero_plan_<timestamp>.gaviero`). No agents run — the output is meant for review before a subsequent `--script` invocation.

---

## 9. Resume / Checkpointing

`--resume` loads `.gaviero/state/{plan_hash}.json` (an `ExecutionState`) and skips nodes marked `Completed`. The plan hash comes from [`CompiledPlan::hash`](../gaviero-core/src/swarm/plan.rs) so changing the script produces a fresh checkpoint file.

---

## 10. Memory Integration

[`MemoryStores::open`](../gaviero-core/src/memory/stores.rs) is best-effort; failure is non-fatal and logged. `--namespace` overrides the write namespace (default: settings → folder name); `--read-ns` adds additional read namespaces (repeatable). `--no-memory` bypasses memory entirely.

Headless contexts cannot prompt interactively, so a pending C1 typed-stores migration aborts the run unless `--accept-c1-migration` is set; the CLI prints the affected DB files plus the proposed backup path. The TUI (`gaviero`) prompts on stdin instead.

`--remember <text>` writes a single memory through [`MemoryServices`](../gaviero-core/src/memory/services.rs) + the writer task (single-consumer invariant). `--remember-scope` overrides the default `repo` scope (`run | module | repo | workspace | global`).

### Memory administration (one-shot, all exit before any agent runs)

| Flag | Purpose |
|---|---|
| `--manifest-last N` / `--manifest-turn <id>` | Print recent `injection_manifests` for audit |
| `--eval-fixture <path>` (+ `--eval-tolerance`, `--eval-report-out`, `--eval-update-baseline`, `--eval-allow-missing-baseline`, `--eval-rerank-ablation`) | Recall@K / MRR regression harness |
| `--eval-from-manifests N` | Cheap rescore replay against persisted manifests |
| `--eval-bootstrap-from-manifests N` | Emit a JSONL fixture from recent manifests |
| `--eval-scope-matrix` (+ `--eval-scope-matrix-scopes <list>`) | Per-scope recall/MRR breakdown |
| `--seed-corpus-from-paths` (+ `--seed-corpus-doc-chars`) | T2 corpus seeding from `gold_must` File entries |
| `--sleep` (+ `--sleep-dry-run`) | Run idle/weekly sleeptime hygiene pass |
| `--utilization-scope <0..4>` (+ `--utilization-top`, `--utilization-asc`) | Print most/least-utilised memories at a level |
| `--deletions-last N` | List `deletions` audit rows |
| `--restore-id <i64>` / `--restore-since <duration>` | Replay deletion(s) through dedup |
| `--forget-query` / `--forget-scope` / `--forget-type` / `--forget-source` (+ `--forget-dry-run`, `--forget-yes`, `--forget-reason`) | Bulk soft-delete; `--forget-yes` required to commit (default is dry-run) |
| `--forget-history-id <i64>` (+ `--redact-confirm REDACT`, `--redact-reason "<text>"`) | C2.4 in-place History redaction (one-way, not undoable) |

---

## 11. Output & Exit Codes

- `--format text` (default): human-readable summary (phases, manifests, merge results, cost, duration).
- `--format json`: structured `SwarmResult` JSON for CI ingestion.
- `--output <path>`: write to a file instead of stdout (also used by `--coordinated`).

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | failure (agent, validation, merge, eval-regression, abort) |
| 2 | argument error (invalid flags) |
| 3 | setup error (workspace / memory init / panic / pending C1 migration) |

---

## 12. Flag Reference

The `Cli` struct in [`src/main.rs`](src/main.rs) is authoritative. Summary by section:

### Input
```
--task <text>              single synthetic WorkUnit (full repo scope)
--work-units <json>        Vec<WorkUnit> as JSON
--script <path>            .gaviero DSL script (include-resolved via compile_file)
--workflow <name>          select workflow (multi-workflow scripts)
--prompt-file <path>       file contents become {{PROMPT}} / default prompt
--var KEY=VALUE            override script-level vars (repeatable, --script only)
--tiers-file <path>        tier-overrides profile (--script only)
```

### Execution
```
--model <spec>             claude:X / codex:X / cursor:X /
                           ollama:X / local:X (provider:model required)
--coordinator-model <spec> planner model for --coordinated
--ollama-base-url <url>    override Ollama endpoint
--auto-accept              skip interactive review
--resume                   resume from checkpoint
--max-retries N            inner-loop retries (default 5)
--attempts N               BestOfN
--test-first               TDD red phase
--no-iterate               single pass
--escalate-after M         escalate tier after M attempts
--max-parallel N           parallel agents (currently sequential)
```

### Memory — scope
```
--namespace <name>         write namespace
--read-ns <name>           additional read namespace (repeatable)
--no-memory                disable memory
--accept-c1-migration      consent to v10 typed-stores split
--remember <text>          one-shot /remember from headless mode
--remember-scope <level>   run / module / repo (default) / workspace / global
```

### Repo map / branch hygiene
```
--graph                    build/update code knowledge graph and exit
--exclude <name|glob>      folder/glob to skip (comma-separated or repeatable)
--cleanup-branches         list/delete leftover gaviero/* branches and exit
--force                    with --cleanup-branches, actually delete
                           (dry-run by default; currently checked-out
                            branch is always skipped; `git worktree prune`
                            runs first)
```

### Coordination
```
--coordinated              emit .gaviero plan only (no execution)
--output <path>            output path for --coordinated (default: tmp/...)
```

### Manifests / eval
```
--manifest-last N
--manifest-turn <id>
--eval-fixture <path> [--eval-tolerance F] [--eval-report-out <path>]
                      [--eval-update-baseline] [--eval-allow-missing-baseline]
                      [--eval-rerank-ablation]
--eval-from-manifests N
--eval-bootstrap-from-manifests N
--eval-scope-matrix [--eval-scope-matrix-scopes <csv>]
--seed-corpus-from-paths [--seed-corpus-doc-chars N]
```

### Hygiene
```
--sleep [--sleep-dry-run]
--utilization-scope 0..4 [--utilization-top N] [--utilization-asc]
```

### Deletions / restore / forget / redact
```
--deletions-last N
--restore-id <i64>
--restore-since <duration>
--forget-query <text>      / --forget-scope <path> / --forget-type <t>
                           / --forget-source <s>
                           [--forget-dry-run | --forget-yes]
                           [--forget-reason "<text>"]
--forget-history-id <i64>  --redact-confirm REDACT --redact-reason "<text>"
```

### Output / diagnostics
```
--format <text|json>       output format
--trace <path>             JSON trace log (enables DEBUG tracing)
--verbose / -v             INFO (-v) / DEBUG (-vv) tracing to stderr
--repo <path>              repository / workspace root (default: .)
```

---

## 13. Error Reporting

Compilation errors (`miette::Report` from [`gaviero-dsl`](../gaviero-dsl)) print with source spans to stderr. Runtime errors surface through observer callbacks; fatal errors use `anyhow::Context` all the way up and exit with the appropriate code. Pending-C1-migration aborts print the affected files and proposed backup path.

---

## 14. Dependencies

- [`gaviero-core`](../gaviero-core), [`gaviero-dsl`](../gaviero-dsl)
- `clap 4` (derive), `tokio`, `serde` / `serde_json`
- `miette` + `anyhow` — diagnostics / error propagation
- `tracing`, `tracing-subscriber`, `dirs`

---

See [CLAUDE.md](CLAUDE.md) for conventions and [README.md](README.md) for end-to-end examples.
