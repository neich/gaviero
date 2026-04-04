# gaviero-core

The execution engine for Gaviero. All agent I/O, multi-agent swarm orchestration, write-gated proposal review, iterative refinement, semantic memory, git integration, and syntax validation live here. The `gaviero-tui` and `gaviero-cli` binaries are thin shells that implement observer traits and call into this library.

---

## What it provides

| Capability | Entry point |
|---|---|
| Run a single agent task with strategy loop | `iteration::IterationEngine::run()` |
| Run a multi-agent swarm from a plan | `swarm::pipeline::execute()` |
| Opus-powered task decomposition to DSL | `swarm::pipeline::plan_coordinated()` |
| Validate file syntax after edits | `validation_gate::ValidationPipeline` |
| Propose, review, and apply file changes | `write_gate::WriteGatePipeline` |
| Semantic memory (store + search) | `memory::init()` → `MemoryStore` |
| Context-aware prompt building (PageRank) | `repo_map::RepoMap` |
| Git operations and worktree management | `git::GitRepo`, `git::WorktreeManager` |
| Workspace settings and namespaces | `workspace::Workspace` |

---

## Key types for callers

```rust
// Describe a task
let unit = WorkUnit {
    id: "my-task".into(),
    description: "Add a hello function to src/lib.rs".into(),
    scope: FileScope {
        owned_paths: vec!["src/".into()],
        read_only_paths: vec![],
        interface_contracts: HashMap::new(),
    },
    model: Some("sonnet".into()),
    tier: ModelTier::Cheap,
    coordinator_instructions: "Write a hello() function that prints 'hello'".into(),
    max_retries: 3,
    ..Default::default()
};

// Wrap in a plan
let plan = CompiledPlan::from_work_units(vec![unit], None);

// Configure execution
let config = SwarmConfig {
    workspace_root: PathBuf::from("."),
    model: "sonnet".into(),
    max_parallel: 1,
    use_worktrees: false,
    read_namespaces: vec![],
    write_namespace: "default".into(),
    context_files: vec![],
};

// Implement the observer traits to receive progress events
struct MyObserver;
impl SwarmObserver for MyObserver { /* ... */ }

// Execute
let result = swarm::pipeline::execute(
    &plan, &config, None, None, &MyObserver, |_| Box::new(MyAcpObserver)
).await?;
```

---

## Iteration strategies

`IterationConfig` controls how the engine refines agent output:

| Strategy | Behaviour |
|---|---|
| `SinglePass` | One attempt, no inner retries |
| `Refine` (default) | One attempt with up to `max_retries` validation-feedback cycles |
| `BestOfN { n }` | n independent attempts; returns the first to pass validation, or the one with the most file changes |

Model escalation: after `escalate_after` failed attempts the engine automatically upgrades from `cheap_model` to `expensive_model`.

---

## Validation gates

Validation runs automatically after every write and feeds failures back to the agent as corrective prompts.

```rust
// Rust workspace: tree-sitter syntax + cargo check
let vp = ValidationPipeline::default_for_rust();

// Any workspace: tree-sitter syntax only (fast)
let vp = ValidationPipeline::fast_only();
```

---

## Semantic memory

```rust
let store = memory::init(None).await?;

// Store a result
store.store_with_options("my-ns", "key", "content", &opts).await?;

// Search (returns ranked results)
let results = store.search_context_filtered(
    &["my-ns", "shared-ns"], "authentication pattern", 10, None
).await?;
```

Embeddings computed locally via ONNX Runtime (nomic-embed-text-v1.5). No network call for memory operations.

---

## Observer pattern

Implement any of the three observer traits to receive real-time progress events:

- `WriteGateObserver` — file proposal lifecycle events
- `AcpObserver` — per-agent streaming events (text chunks, tool calls, validation results)
- `SwarmObserver` — swarm-level events (phase changes, agent status, cost updates)

Observers are passed as `&dyn Trait` to `execute()` and `run_backend()`, enabling zero-copy event routing with no lock contention.

---

## Supported languages (tree-sitter)

Rust, JavaScript, TypeScript, Python, Java, Kotlin, Bash, C, C++, HTML, CSS, JSON, YAML, TOML, LaTeX.
