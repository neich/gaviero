# gaviero-cli — Architecture

Headless runner. `clap` front end + observer wiring; all runtime work delegates to `gaviero-core` and `gaviero-dsl`.

Binary: `gaviero-cli`

---

## 1. Design

Intentionally minimal:
- Parse flags with `clap` (single `Cli` struct, derive macro).
- Pick exactly one operating mode — execution, coordinated planning, repo-map build, eval, sleeptime, memory administration, or `/remember`.
- Wire stderr observers (`AcpObserver`, `SwarmObserver`, write-gate logging).
- Delegate to `gaviero_core` / `gaviero_dsl`.
- Write results to stdout (text or JSON) unless `--output <path>` is set.

Single source file: `src/main.rs` (~2.1 KLOC) containing `Cli`, `CliAcpObserver`, `CliSwarmObserver`, `OutputFormat`, mode dispatch, and `run()`. Tests in `tests/remember_cli.rs`.

The `Cli` struct is the authoritative flag list — read it before adding flags here.

---

## 2. Execution Flow

```
parse Cli (clap)
   │
   ├─ open Workspace + settings (`gaviero_core::workspace`)
   ├─ probe C1 migration; refuse without --accept-c1-migration
   ├─ init MemoryStores (best-effort; --no-memory bypasses)
   │
   ├─ admin / one-shot modes (each exits before agent dispatch):
   │     --remember <text>                 → WriterHandle store, exit
   │     --graph                           → repo-map build/refresh, stats, exit
   │     --manifest-last / --manifest-turn → print injection_manifests, exit
   │     --eval-fixture / --eval-from-… /  → run eval harness, write report, exit
   │       --eval-bootstrap-from-manifests
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
   │     --script <path>        → gaviero_dsl::compile_with_vars(...)
   │
   ├─ apply iteration overrides (--max-retries, --attempts,
   │                             --test-first, --escalate-after, --no-iterate)
   │
   ├─ --coordinated?
   │     yes → swarm::coordinator::plan_coordinated → write .gaviero, exit
   │     no  → swarm::pipeline::execute
   │
   └─ print SwarmResult (text or JSON) → exit(0/1)
```

---

## 3. Input Modes

### `--task <text>`

Creates a synthetic `WorkUnit` with scope `owned_paths = ["."]`, `tier = Cheap`, `privacy = Public`, model from `--model` or workspace default. Wrapped in a single-node `CompiledPlan::from_work_units(vec![unit])`.

### `--work-units <json>`

`serde_json::from_str::<Vec<WorkUnit>>` then `CompiledPlan::from_work_units`.

### `--script <path>`

```rust
let source = fs::read_to_string(path)?;
let plan = gaviero_dsl::compile_with_vars(
    &source, path, workflow, runtime_prompt, &override_vars,
)?;
```

- `--workflow <name>` selects a workflow when the script declares multiple.
- `--prompt-file <path>` reads file contents and supplies them as `runtime_prompt` (replaces every `{{PROMPT}}`; also acts as the full prompt for agents without a `prompt` field).
- `--var KEY=VALUE` (repeatable) overrides entries in the script's top-level `vars {}`. Precedence: agent-level `vars {}` > CLI `--var` > script-level `vars {}`.

---

## 4. Observers

### `CliAcpObserver`

Prints stream chunks (verbose only), tool calls, validation results, retries, and deferred proposals to stderr. Lines are prefixed `[{agent_id}]` for multi-agent tracing.

### `CliSwarmObserver`

Prints phase, agent state, tier start/dispatch, merge conflicts, cost updates, and completion to stderr.

All observer events route to stderr so stdout stays clean for structured output (`--format json`).

---

## 5. Model Resolution

Model spec passed via `--model` (or the DSL `client` decl) is parsed by `gaviero_core::swarm::backend::shared::backend_config_for_model`. Specs are required to be in canonical `provider:model` form — `validate_model_spec` rejects bare names without a provider prefix.

```
claude:<name>               → Claude API (ACP)
ollama:<name>               → Ollama HTTP SSE
local:<name>                → Ollama HTTP SSE (alias)
codex:<name>                → Codex exec
codex-app:<name>            → Codex app-server
```

`--coordinator-model` picks the planner model for `--coordinated`. `--ollama-base-url` overrides the Ollama endpoint (defaults to workspace setting, then `http://localhost:11434`).

`TierRouter` maps `(ModelTier, PrivacyLevel)` to a concrete backend; `PrivacyScanner` can promote a unit to `LocalOnly` based on glob matches against sensitive paths.

---

## 6. Iteration Overrides

| Flag | Effect on `IterationConfig` |
|---|---|
| `--max-retries N` | inner-loop retries per attempt (default 5) |
| `--attempts N` | independent attempts → `Strategy::BestOfN(N)` |
| `--test-first` | TDD red phase before the edit loop |
| `--escalate-after M` | escalate tier after M failed attempts |
| `--no-iterate` | single pass (overrides `--max-retries`) |

Applied after the `CompiledPlan` is produced, before `pipeline::execute`.

---

## 7. Coordinated Mode

```
gaviero-cli --task "..." --coordinated --model claude:opus
```

Calls `swarm::coordinator::plan_coordinated(task, workspace, memory, coordinator_model)`, which emits a `CompiledPlan`. The plan is serialised back to `.gaviero` form and written to `--output` (or `tmp/gaviero_plan_<timestamp>.gaviero`). No agents run — the output is meant for review before a subsequent `--script` invocation.

---

## 8. Resume / Checkpointing

`--resume` loads `.gaviero/state/{plan_hash}.json` (an `ExecutionState`) and skips nodes marked `Completed`. The plan hash comes from `CompiledPlan::hash` so changing the script produces a fresh checkpoint file.

---

## 9. Memory Integration

`MemoryStores::open` is best-effort; failure is non-fatal and logged. `--namespace` overrides the write namespace (default: settings → folder name); `--read-ns` adds additional read namespaces (repeatable). `--no-memory` bypasses memory entirely.

Headless contexts cannot prompt interactively, so a pending C1 typed-stores migration aborts the run unless `--accept-c1-migration` is set; the CLI prints the affected DB files plus the proposed backup path. The TUI (`gaviero`) prompts on stdin instead.

`--remember <text>` writes a single memory through `MemoryServices` + the writer task (single-consumer invariant). `--remember-scope` overrides the default `repo` scope (`run | module | repo | workspace | global`).

### Memory administration (one-shot, all exit before any agent runs)

| Flag | Purpose |
|---|---|
| `--manifest-last N` / `--manifest-turn <id>` | Print recent `injection_manifests` for audit |
| `--eval-fixture <path>` (+ `--eval-tolerance`, `--eval-report-out`, `--eval-update-baseline`, `--eval-allow-missing-baseline`, `--eval-rerank-ablation`) | Recall@K / MRR regression harness |
| `--eval-from-manifests N` | Cheap rescore replay against persisted manifests |
| `--eval-bootstrap-from-manifests N` | Emit a JSONL fixture from recent manifests |
| `--sleep` (+ `--sleep-dry-run`) | Run idle/weekly sleeptime hygiene pass |
| `--utilization-scope <0..4>` (+ `--utilization-top`, `--utilization-asc`) | Print most/least-utilised memories at a level |
| `--deletions-last N` | List `deletions` audit rows |
| `--restore-id <i64>` / `--restore-since <duration>` | Replay deletion(s) through dedup |
| `--forget-query` / `--forget-scope` / `--forget-type` / `--forget-source` (+ `--forget-dry-run`, `--forget-yes`, `--forget-reason`) | Bulk soft-delete; `--forget-yes` required to commit (default is dry-run) |
| `--forget-history-id <i64>` (+ `--redact-confirm REDACT`, `--redact-reason "<text>"`) | C2.4 in-place History redaction (one-way, not undoable) |

---

## 10. Output & Exit Codes

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

## 11. Flag Reference

The `Cli` struct in `src/main.rs` is authoritative. Summary by section:

### Input
```
--task <text>              single synthetic WorkUnit (full repo scope)
--work-units <json>        Vec<WorkUnit> as JSON
--script <path>            .gaviero DSL script
--workflow <name>          select workflow (multi-workflow scripts)
--prompt-file <path>       file contents become {{PROMPT}} / default prompt
--var KEY=VALUE            override script-level vars (repeatable)
```

### Execution
```
--model <spec>             claude:X / codex:X / codex-app:X /
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

### Repo map
```
--graph                    build/update code knowledge graph and exit
--exclude <name|glob>      folder/glob to skip (comma-separated or repeatable)
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

## 12. Error Reporting

Compilation errors (`miette::Report` from `gaviero-dsl`) print with source spans to stderr. Runtime errors surface through observer callbacks; fatal errors use `anyhow::Context` all the way up and exit with the appropriate code. Pending-C1-migration aborts print the affected files and proposed backup path.

---

## 13. Dependencies

- `gaviero-core`, `gaviero-dsl`
- `clap` (derive), `tokio`, `serde` / `serde_json`
- `miette` + `anyhow` — diagnostics / error propagation

---

See [CLAUDE.md](CLAUDE.md) for conventions and [README.md](README.md) for end-to-end examples.
