# gaviero-cli — Architecture

`gaviero-cli` is a single-binary headless runner. It holds no domain logic; its entire job is to parse command-line arguments, delegate to `gaviero-core` and `gaviero-dsl`, and write results to stdout/stderr. The full execution engine lives in `gaviero-core`.

---

## Module map

The crate is a single file:

| File | Purpose |
|---|---|
| `main.rs` (294 lines) | Entry point, `Cli` struct, argument parsing, execution dispatch, observer implementations |

No sub-modules exist. Logic that would warrant a sub-module belongs in `gaviero-core`.

---

## `Cli` struct (clap derive)

```rust
struct Cli {
    repo:         PathBuf,          // --repo       default "."
    task:         Option<String>,   // --task
    work_units:   Option<String>,   // --work-units  (JSON array)
    script:       Option<PathBuf>,  // --script      (.gaviero file)
    auto_accept:  bool,             // --auto-accept
    max_parallel: usize,            // --max-parallel  default 1
    model:        String,           // --model       default "sonnet"
    namespace:    Option<String>,   // --namespace
    read_ns:      Vec<String>,      // --read-ns     (repeatable)
    format:       String,           // --format      "text" | "json"
    coordinated:  bool,             // --coordinated (requires --task)
    resume:       bool,             // --resume      load checkpoint; skip completed nodes
    trace:        Option<PathBuf>,  // --trace       write structured JSON trace to file
}
```

**Mutual exclusions (clap groups):**
- `--task` conflicts with `--work-units` and `--script`
- `--work-units` conflicts with `--task` and `--script`
- `--coordinated` requires `--task` and is incompatible with `--script`

---

## Execution flow

```
main()
  │
  ├─ 1. Parse Cli args (clap)
  ├─ 2. Canonicalize --repo path
  ├─ 3. Workspace::single_folder(repo)   ← load settings
  ├─ 4. Resolve namespaces               ← see Namespace resolution
  ├─ 5. memory::init(None).await         ← graceful degradation on failure
  │
  ├─ 6. Resolve work units (mutually exclusive):
  │       --script  → gaviero_dsl::compile(source, filename, None, None)
  │       --task    → synthetic WorkUnit { owned_paths=["."], model }
  │       --work-units → serde_json::from_str::<Vec<WorkUnit>>(json)
  │
  ├─ 7. Build SwarmConfig { workspace_root, model, max_parallel, … }
  │
  ├─ [--coordinated path]
  │       plan_coordinated(task, config, coord_config, memory, observer)
  │         → Coordinator::plan_as_dsl() → Opus generates .gaviero DSL text
  │         → write to tmp/gaviero_plan_<timestamp>.gaviero
  │         → print path to stdout
  │         → eprintln review instructions to stderr
  │         → return Ok(())   ← EXIT; no agent execution
  │
  └─ [normal path]
        build SwarmConfig { …, resume: cli.resume, trace_path: cli.trace }
        execute(work_units, config, memory, observer, make_obs)
          → SwarmResult
          → format output (text | json) to stdout
          → exit 0 if success, exit 1 if any agent failed
```

---

## Coordinated planning path

When `--coordinated` is set:

```
plan_coordinated(
    task,
    &config,
    CoordinatorConfig { model: "opus", .. },
    memory,
    &CliSwarmObserver,          ← prints [coordinator] lines to stderr
    |_| Box::new(CliAcpObserver),
) → Result<String>              ← .gaviero DSL text

timestamp = SystemTime::now().as_secs()
plan_path = workspace_root / "tmp" / "gaviero_plan_{timestamp}.gaviero"
fs::create_dir_all(plan_path.parent())
fs::write(plan_path, dsl_text)

// stdout: path only (pipeable)
println!("{}", plan_path.display())

// stderr: human instructions
eprintln!("[plan] saved to {}", plan_path.display())
eprintln!("[plan] review it, then run with:")
eprintln!("         gaviero --script {}", plan_path.display())

return Ok(())   // agents do NOT run
```

The plan path on stdout enables shell capture: `plan=$(gaviero --coordinated --task "…")`.

---

## Observer implementations

### `CliSwarmObserver`

Implements `SwarmObserver`. All output goes to **stderr**.

| Method | Output |
|---|---|
| `on_phase_changed(phase)` | `[phase] <phase>` |
| `on_agent_state_changed(id, status, detail)` | `[agent:<id>] <Status>: <detail>` |
| `on_tier_started(current, total)` | `[tier] <current>/<total>` |
| `on_merge_conflict(branch, files)` | `[conflict] branch=<b>  files=<f>` |
| `on_completed(result)` | `[completed] success=<bool>` |
| `on_coordination_started(prompt)` | `[coordinator] planning: <first 80 chars>` |
| `on_coordination_complete(n, summary)` | `[coordinator] planned <n> units: <summary>` |
| `on_tier_dispatch(id, tier, backend)` | `[dispatch] <id>  tier=<t>  backend=<b>` |
| `on_cost_update(estimate)` | `[cost] ~$<amount>` |

### `CliAcpObserver`

Implements `AcpObserver`. Streaming text is suppressed; only tool calls and completion are printed to **stderr**.

| Method | Output |
|---|---|
| `on_stream_chunk` | no-op |
| `on_streaming_status` | no-op |
| `on_tool_call_started(_, tool)` | `  [tool] <tool>` |
| `on_message_complete(_, role, _)` | `  [done] <role>` |
| `on_proposal_deferred(_, path, +, -)` | `  [deferred] <path>  +<n>/-<n>` |

The `make_obs` closure passed to `execute()` is `|_agent_id| Box::new(CliAcpObserver)` — the same observer is used for every agent.

---

## Namespace resolution

**Write namespace** (priority order, first match wins):
1. `--namespace` flag
2. Workspace settings (`agent.namespace`)
3. Folder name (basename of `--repo`)

**Read namespaces** (additive merge):
1. Start with workspace `agent.read_namespaces` setting (empty list if absent)
2. Append all `--read-ns` flag values
3. Prepend the write namespace (ensures the run can read its own prior output)
4. Deduplicate (order preserved)

```
Logged to stderr on startup:
  [namespace] write=<write_ns>, read=[<ns1>, <ns2>, …]
```

---

## stdout / stderr split

| Stream | Content |
|---|---|
| **stderr** | All telemetry: `[phase]`, `[agent:…]`, `[coordinator]`, `[tool]`, `[done]`, `[cost]`, `[namespace]`, `[memory]`, `[plan]`, error messages |
| **stdout** | Results only: SwarmResult manifests (text or JSON) **or** plan file path (coordinated mode) |

This split enables clean piping and redirection:
```bash
result=$(gaviero --format json --task "…")   # capture manifests
plan=$(gaviero --coordinated --task "…")     # capture plan path
gaviero --task "…" 2>/dev/null               # suppress telemetry
gaviero --task "…" > /dev/null               # suppress results, keep telemetry
```

---

## Integration

### Imports from `gaviero-core`

| Symbol | Used for |
|---|---|
| `Workspace::single_folder` | Load settings |
| `memory::init` | Initialise `MemoryStore` |
| `swarm::pipeline::execute` | Run work units |
| `swarm::pipeline::plan_coordinated` | Coordinated planning |
| `swarm::pipeline::SwarmConfig` | Execution configuration |
| `swarm::coordinator::CoordinatorConfig` | Opus model config |
| `swarm::models::{WorkUnit, AgentStatus}` | Work unit definition and result |
| `observer::{SwarmObserver, AcpObserver}` | Observer trait implementations |

### Imports from `gaviero-dsl`

| Symbol | Used for |
|---|---|
| `compile(source, filename, None, None)` | Compile `--script` file to `Vec<WorkUnit>` |

### Key dependencies

| Crate | Purpose |
|---|---|
| `clap` | Argument parsing (derive macros) |
| `tokio` | Async runtime (single-threaded `#[tokio::main]`) |
| `anyhow` | Error propagation |
| `serde_json` | JSON output for `--format json`; parsing `--work-units` |
| `miette` | Pretty DSL error formatting (passed through from `gaviero-dsl`) |
| `tracing` / `tracing-subscriber` | WARN-level structured logging to stderr |

---

## Design decisions

1. **No sub-modules.** A 294-line `main.rs` is intentional — CLI dispatch logic is not complex enough to warrant splitting. New features belong in `gaviero-core` or `gaviero-dsl`.

2. **Three work-unit sources.** `--task`, `--work-units`, and `--script` cover the full spectrum from ad-hoc interactive use to scripted CI pipelines, without inventing a new intermediate format.

3. **Coordinated mode exits after planning.** `--coordinated` never runs agents. The two-step flow (plan → review → `--script`) makes phantom file references visible before any agent executes.

4. **Graceful memory degradation.** If `memory::init()` fails (model not downloaded, disk full, permissions), the CLI logs a warning and continues with `memory = None`. Agents run without memory context rather than aborting.

5. **stderr for telemetry, stdout for results.** This is a Unix pipeline convention. It allows result capture, silent mode, and log redirection independently.

6. **`make_obs` factory per agent.** `execute()` accepts a closure that produces one `AcpObserver` per agent. The CLI uses a constant closure, but the pattern allows future per-agent observer customisation (e.g. per-agent log files).
