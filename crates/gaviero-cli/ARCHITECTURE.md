# gaviero-cli — Architecture

A single-binary headless runner. Holds no domain logic: its job is to parse command-line arguments, delegate to `gaviero-core` and `gaviero-dsl`, and write results to stdout/stderr.

---

## File structure

```
gaviero-cli/src/
└── main.rs     ~450 lines — everything lives here
```

All execution, iteration, validation, memory, and swarm logic is in `gaviero-core`. All DSL compilation is in `gaviero-dsl`.

---

## The `Cli` struct (clap derive)

```
--repo            PathBuf           workspace root (default: ".")
--task            Option<String>    single task → one synthetic WorkUnit
--work-units      Option<String>    JSON array of WorkUnit definitions
--script          Option<PathBuf>   path to .gaviero DSL file

--auto-accept     bool              accept all writes without review
--max-parallel    usize             concurrent agents (default: 1)
--model           sonnet|opus|haiku model for --task mode (default: sonnet)
--namespace       Option<String>    override write namespace
--read-ns         Vec<String>       additional read namespaces (repeatable)
--format          text|json         output format (default: text)
--coordinated     bool              Opus planning mode — writes DSL file, then exits
--resume          bool              load execution checkpoint, skip completed nodes
--output          Option<PathBuf>   output path for --coordinated DSL plan

--max-retries     u32               inner-loop retries per attempt (default: 5)
--attempts        u32               BestOfN attempt count when > 1 (default: 1)
--test-first      bool              generate failing tests before editing
--no-iterate      bool              force SinglePass strategy

--trace           Option<PathBuf>   write DEBUG-level JSON trace log to this file
--graph           bool              build/update code knowledge graph and exit
```

Mutual exclusions (enforced by clap): `--task` conflicts with `--work-units` and `--script`. `--coordinated` requires `--task`. `--output` requires `--coordinated`.

---

## Execution flow

```
main()
  1. Parse Cli args
  2. Canonicalise --repo
  3. Load Workspace (settings, namespace resolution)
  4. Init MemoryStore (graceful failure → memory = None)

  [--graph path]
     → build/update code knowledge graph (repo_map/graph_builder)
     → print stats (files scanned/changed/removed, total nodes/edges)
     → exit

  5. Build CompiledPlan (one of three input modes):
     --script      → gaviero_dsl::compile(source, file, None, None)
     --task        → synthetic WorkUnit with owned=["."], max_retries=1
     --work-units  → serde_json::from_str::<Vec<WorkUnit>>(json)
                     → CompiledPlan::from_work_units(units)

  6. Apply iteration CLI flags to plan.iteration_config:
       --no-iterate  → strategy = SinglePass
       --attempts N  → strategy = BestOfN { n: N }  (if N > 1)
       --max-retries → iteration_config.max_retries
       --test-first  → iteration_config.test_first = true

  [--coordinated path]
     → pipeline::plan_coordinated()    (Opus generates DSL)
     → write to --output path or tmp/gaviero_plan_<ts>.gaviero
     → print plan path to stdout
     → print review instructions to stderr
     → exit (no agent execution)

  [normal path]
     → load checkpoint if --resume
     → pipeline::execute(plan, config, checkpoint, memory, observer, make_obs)
     → print SwarmResult to stdout (text or JSON)
     → exit 0 on success, 1 on failure
```

---

## Observer implementations

### `CliSwarmObserver` — implements `SwarmObserver`

All output to **stderr**:

| Method | Output |
|---|---|
| `on_phase_changed(phase)` | `[phase] <phase>` |
| `on_agent_state_changed(id, status, detail)` | `[agent:<id>] <status> <detail>` |
| `on_tier_started(cur, tot)` | `[tier] <cur>/<tot>` |
| `on_merge_conflict(branch, files)` | `[conflict] branch=<b> files=<f>` |
| `on_completed(result)` | `[completed] success=<bool>` |
| `on_coordination_started(prompt)` | `[coordinator] planning: <80 chars>` |
| `on_tier_dispatch(id, tier, backend)` | `[dispatch] <id> tier=<t> backend=<b>` |
| `on_cost_update(est)` | `[cost] ~$<usd>` |

### `CliAcpObserver` — implements `AcpObserver`

Streaming text suppressed. Tool calls and completion to **stderr**:

| Method | Output |
|---|---|
| `on_stream_chunk` | (suppressed) |
| `on_streaming_status` | (suppressed) |
| `on_tool_call_started(tool)` | `  [tool] <tool>` |
| `on_message_complete(role, _)` | `  [done]` (assistant only) |
| `on_proposal_deferred(path, …)` | `  [deferred] <path>` |
| `on_validation_result(gate, pass, msg)` | `  [validation] <gate>: pass\|fail — <msg>` |
| `on_validation_retry(attempt, max)` | `  [retry] attempt <n>/<max>` |

---

## Namespace resolution

**Write namespace** priority:
1. `--namespace` flag
2. Workspace settings (`agent.namespace`)
3. Folder name (basename of `--repo`)

**Read namespaces** (additive):
1. Start with workspace settings base list
2. Append all `--read-ns` values (dedup)
3. Prepend write namespace (always readable)

Logged at startup: `[namespace] write=<ns>, read=[<ns1>, …]`

---

## Coordinated mode (`--coordinated`)

Implements the "plan → review → execute" two-step:

1. `pipeline::plan_coordinated()` sends the task to Opus
2. Opus decomposes the task and returns `.gaviero` DSL text
3. DSL text written to `--output` path or `tmp/gaviero_plan_<unix-ts>.gaviero`
4. **stdout:** plan path only (pipeable)
5. **stderr:** human-readable review instructions
6. **exits without running any agents**

The user inspects and optionally edits the plan, then runs:
```bash
gaviero-cli --script tmp/gaviero_plan_<ts>.gaviero
```

---

## Output formats

### `--format text` (default)

```
task-0: OK (src/auth.rs, tests/auth_test.rs)
task-1: FAIL: validation failed
```

### `--format json`

```json
{
  "success": true,
  "manifests": [
    {
      "work_unit_id": "task-0",
      "status": "Completed",
      "modified_files": ["src/auth.rs"]
    }
  ]
}
```

---

## Tracing (`--trace <file>`)

Enables DEBUG-level structured JSON logging to the specified file. Use for CI post-mortems and timing analysis. Without `--trace`, only WARN+ logs go to stderr.

---

## Code knowledge graph (`--graph`)

Builds or updates the SQLite-backed code knowledge graph via `repo_map/graph_builder.rs`. Prints stats:
```
files_scanned: 42, files_changed: 3, files_unchanged: 39
files_removed: 0, total_nodes: 187, total_edges: 412
```

Useful after major codebase changes before running agent tasks that use `impact_scope` or `context` blocks.

---

## Integration surface

| Import | From |
|---|---|
| `swarm::pipeline::{execute, plan_coordinated}` | gaviero-core |
| `swarm::pipeline::SwarmConfig` | gaviero-core |
| `swarm::coordinator::CoordinatorConfig` | gaviero-core |
| `swarm::models::{WorkUnit, AgentStatus, SwarmResult}` | gaviero-core |
| `swarm::plan::CompiledPlan` | gaviero-core |
| `memory::init`, `MemoryStore` | gaviero-core |
| `workspace::Workspace` | gaviero-core |
| `observer::{SwarmObserver, AcpObserver}` | gaviero-core |
| `repo_map::graph_builder` | gaviero-core |
| `gaviero_dsl::compile` | gaviero-dsl |

---

## Concurrency model

Single tokio runtime. All work is async. No threads spawned directly. Agent parallelism is managed by the swarm pipeline's `Semaphore`.

---

## Error handling strategy

- `anyhow::Result` for all fallible operations
- Memory init failure: logs warning, continues with `memory = None`
- DSL compilation errors: printed via miette diagnostics, exit 1
- Swarm execution errors: printed to stderr, exit 1
- `--trace` logging failure: warns and continues without trace

---

## Design decisions

1. **Single file.** ~450-line `main.rs`. Complex logic belongs in `gaviero-core`; duplication here signals a missing abstraction in the library.
2. **stderr for telemetry, stdout for results.** Enables clean piping: `plan=$(gaviero-cli --coordinated --task "…")`.
3. **Graceful memory degradation.** `memory::init()` failure logs a warning and proceeds with `memory = None`.
4. **`make_obs` factory per agent.** `execute()` accepts a closure that produces one `AcpObserver` per agent ID. CLI always returns `Box::new(CliAcpObserver)`; the indirection allows future per-agent customisation.
5. **Iteration flags override both DSL and defaults.** CLI flags are applied after plan compilation, so `--no-iterate` overrides even a `strategy refine` in a `.gaviero` file.
6. **`--graph` is standalone.** Graph building exits immediately without agent execution, making it safe to run in CI pre-steps.
