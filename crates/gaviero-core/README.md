# gaviero-core

The shared execution engine for Gaviero. All agent I/O, multi-agent swarm orchestration, write-gated proposal review, semantic memory, git integration, and syntax analysis live here. The `gaviero-tui` and `gaviero-cli` binaries are thin shells that implement observer traits and call into this library.

---

## Key capabilities

- **Workspace configuration** — multi-root settings cascade (user → workspace → folder); resolves models, namespaces, layout presets
- **Agent I/O via ACP** — spawn `claude` subprocess, stream NDJSON events, route proposed file writes through the write gate
- **Write-gated proposals** — every agent file write is staged as a diff; hunks are accepted or rejected individually before anything reaches disk
- **Swarm orchestration** — coordinator decomposes a task into a `.gaviero` plan file; a parallel pipeline executes agents in dependency tiers with git worktree isolation
- **Semantic memory** — SQLite + ONNX embeddings (768d); per-namespace context retrieval enriches agent prompts across runs
- **Git + tree-sitter** — git repo operations, worktree lifecycle, 16+ language syntax parsers for hunk enrichment and auto-indent

---

## Adding to a project

```toml
[dependencies]
gaviero-core = { path = "../gaviero-core" }
```

The ONNX embedding model (`nomic-embed-text-v1.5`) is downloaded to `~/.cache/gaviero/` on first use. Memory features degrade gracefully if the download fails.

---

## Primary entry points

| Symbol | What it does |
|---|---|
| `Workspace::load(path)` | Load settings from `.gaviero-workspace`; resolve model names and namespaces |
| `memory::init(None)` | Initialise `MemoryStore` (async; returns `None` on failure) |
| `swarm::pipeline::plan_coordinated(…)` | Opus writes a `.gaviero` DSL plan file via `plan_as_dsl()` (no execution) |
| `swarm::pipeline::execute(work_units, …)` | Run a `Vec<WorkUnit>` → `SwarmResult` |
| `AcpPipeline::send_prompt(…)` | Single-agent chat turn; proposals routed through write gate |
| `WriteGatePipeline::new(mode, observer)` | Create a write gate in Interactive / AutoAccept / Deferred mode |
| `GitRepo::open(path)` | Open a git repository; provides status, branch, diff, worktree operations |

---

## Observer pattern

Core execution fires callbacks rather than blocking on UI. Implement any of the three traits and pass them to `execute()` or `AcpPipeline`:

```rust
use gaviero_core::observer::{WriteGateObserver, AcpObserver, SwarmObserver};

struct MyObserver;

impl SwarmObserver for MyObserver {
    fn on_phase_changed(&self, phase: &str) {
        eprintln!("[{}]", phase);
    }
    fn on_agent_state_changed(&self, id: &str, status: &AgentStatus, detail: &str) {
        eprintln!("  {}: {:?} — {}", id, status, detail);
    }
    // … remaining methods have default no-op implementations
}
```

All three traits provide default no-op implementations for every method, so you only override what you need.

| Trait | Events |
|---|---|
| `WriteGateObserver` | Proposal created / hunk updated / file written to disk |
| `AcpObserver` | Streaming text chunk / tool call / message complete / deferred proposal |
| `SwarmObserver` | Phase change / agent status / tier progress / merge conflict / completion |

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full method list of each trait.

---

## Typical usage pattern

```rust
use std::sync::Arc;
use gaviero_core::{
    workspace::Workspace,
    memory,
    swarm::pipeline::{self, SwarmConfig},
    swarm::models::WorkUnit,
    observer::SwarmObserver,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Load workspace settings
    let ws = Workspace::single_folder(".")?;
    let model = ws.resolve_setting("agent.model", None)
        .unwrap_or_else(|| "sonnet".into());

    // 2. Initialise memory (optional)
    let memory = memory::init(None).await.ok().map(Arc::new);

    // 3. Build work units — or compile a .gaviero file:
    //    let plan: CompiledPlan = gaviero_dsl::compile(source, "task.gaviero", None, None)?;
    //    let units = plan.work_units_ordered();
    let units = vec![WorkUnit {
        id: "task".into(),
        coordinator_instructions: "Fix the failing test".into(),
        // … other fields …
        ..Default::default()
    }];

    // 4. Execute
    let config = SwarmConfig {
        workspace_root: ".".into(),
        model,
        max_parallel: 1,
        use_worktrees: false,
        read_namespaces: vec![],
        write_namespace: "my-project".into(),
        context_files: vec![],
    };

    let observer = MyObserver;
    let result = pipeline::execute(
        units,
        &config,
        memory,
        &observer,
        |_| Box::new(MyObserver),
    ).await?;

    println!("success: {}", result.success);
    Ok(())
}
```

---

## Build notes

- **Rust edition:** 2024 (requires Rust 1.84+)
- **ONNX model:** Downloaded on first `memory::init()` call; requires internet access. Subsequent runs use the cached model at `~/.cache/gaviero/`.
- **tree-sitter grammars:** Bundled as `build.rs` compiled C libraries. No external toolchain required beyond a C compiler.
- **`sqlite-vec`:** Linked statically; no system SQLite dependency.
