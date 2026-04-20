# gaviero-cli — Architecture

Headless runner. `clap` front end + observer wiring; all runtime work delegates to `gaviero-core` and `gaviero-dsl`.

---

## 1. Design

Intentionally minimal:
- Parse flags with `clap`.
- Construct one input plan (one of four shapes).
- Wire `AcpObserver` / `SwarmObserver` to stderr.
- Delegate to `gaviero_core::swarm::pipeline::execute` or `swarm::coordinator`.
- Write results to stdout (text or JSON).

Single source file: `src/main.rs` (~1 KLOC) containing `Cli`, `CliAcpObserver`, `CliSwarmObserver`, and `run()`.

---

## 2. Execution Flow

```
parse Cli (clap)
   │
   ├─ open Workspace + settings (`gaviero_core::workspace`)
   ├─ init MemoryStore (best-effort; continues on failure)
   │
   ├─ input mode:
   │     ┌─ --task <text>          → synthetic WorkUnit (full repo scope)
   │     ├─ --work-units <json>    → Vec<WorkUnit> (serde)
   │     ├─ --script <path>        → gaviero_dsl::compile_with_vars(...)
   │     └─ --graph                → build/update repo-map, print stats, exit
   │
   ├─ apply iteration overrides  (--max-retries, --attempts,
   │                              --test-first, --escalate-after, --no-iterate)
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

Creates a synthetic `WorkUnit` with scope `owned_paths = ["."]`, `tier=Cheap`, `privacy=Public`, model from `--model` or workspace default. Wrapped in a single-node `CompiledPlan::from_work_units(vec![unit])`.

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
- `--prompt-file <path>` reads the file's contents and supplies them as `runtime_prompt` (substituted for every `{{PROMPT}}`; also used as the full prompt for agents without a `prompt` field).
- `--var KEY=VALUE` (repeatable) overrides entries in the script's top-level `vars {}`; CLI overrides beat script-level vars but not agent-level vars.

### `--graph`

Builds or refreshes the code knowledge graph (`gaviero_core::repo_map`), prints statistics, then exits without running agents. Honours `--exclude` (comma-separated or repeated, bare names match basename at any depth; `/`-containing values are globs).

---

## 4. Observers

### `CliAcpObserver`

Prints stream chunks, tool calls, validation results, retries, and deferred proposals to stderr. Mirrors the agent-chat output the TUI renders, so CI logs are readable.

### `CliSwarmObserver`

Prints phase, agent state, tier start/dispatch, merge conflicts, cost updates, and completion to stderr.

All events route to stderr so stdout stays clean for structured output (`--format json`).

---

## 5. Model Resolution

Model spec passed via `--model` (or the DSL `client` decl) is parsed by `gaviero_core::swarm::backend::shared::backend_config_for_model`:

```
sonnet | opus | haiku       → claude:<same>
claude:<name>               → Claude API (ACP)
ollama:<name>               → Ollama HTTP SSE
codex:<name>                → Codex (codex exec / app-server)
local:<url>                 → OpenAI-compatible endpoint
```

Validation via `validate_model_spec`. `--coordinator-model` picks the planner model for `--coordinated`. `--ollama-base-url` overrides the Ollama endpoint (defaults to workspace setting, then `http://localhost:11434`).

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

Calls `swarm::coordinator::plan_coordinated(task, workspace, memory, coordinator_model)` which emits a `CompiledPlan`. The plan is serialised back to `.gaviero` form and written to `--output` or `tmp/gaviero_plan_<timestamp>.gaviero`. No agents run — the output is meant for review before a subsequent `--script` invocation.

---

## 8. Resume / Checkpointing

`--resume` loads `.gaviero/state/{plan_hash}.json` (an `ExecutionState`) and skips nodes marked `Completed`. The plan hash comes from `CompiledPlan::hash` so changing the script produces a fresh checkpoint file.

---

## 9. Memory Integration

`MemoryStore::init` is best-effort; failure is non-fatal and logged. `--namespace` overrides the write namespace (default: settings → folder name), `--read-ns` adds additional read namespaces (repeatable). `--no-memory` bypasses memory entirely.

---

## 10. Output & Exit Codes

- `--format text` (default): human-readable summary (phases, manifests, merge results, cost, duration).
- `--format json`: structured `SwarmResult` JSON for CI ingestion.
- `--output <path>`: write to a file instead of stdout.

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | failure (agent, validation, merge, or abort) |
| 2 | argument error (invalid flags) |
| 3 | setup error (workspace / memory init / panic) |

---

## 11. Flag Reference

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
--model <spec>             sonnet / opus / claude:X / codex:X / ollama:X / local:URL
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

### Memory
```
--namespace <name>         write namespace
--read-ns <name>           additional read namespace (repeatable)
--no-memory                disable memory
```

### Repo map
```
--graph                    build/update code knowledge graph and exit
--exclude <glob>           folder/glob to skip (comma-separated or repeatable)
```

### Coordination
```
--coordinated              emit .gaviero plan only (no execution)
--output <path>            output path for --coordinated (default: tmp/...)
```

### Output / diagnostics
```
--format <text|json>       output format
--trace <path>             JSON trace log (enables DEBUG tracing)
--repo <path>              repository / workspace root (default: .)
```

---

## 12. Error Reporting

Compilation errors (`miette::Report` from `gaviero-dsl`) print with source spans to stderr. Runtime errors surface through observer callbacks; fatal errors use `anyhow::Context` all the way up and exit with the appropriate code.

---

## 13. Dependencies

- `gaviero-core`, `gaviero-dsl`
- `clap` (derive), `tokio`, `serde` / `serde_json`
- `miette` + `anyhow` — diagnostics / error propagation

---

See [CLAUDE.md](CLAUDE.md) for conventions and [README.md](README.md) for end-to-end examples.
