# gaviero-core

The execution engine for Gaviero. All agent I/O, multi-agent swarm orchestration, write-gated proposal review, iterative refinement, semantic memory, code knowledge graph, git integration, and syntax validation live here. The `gaviero-tui` and `gaviero-cli` binaries are thin shells that implement observer traits and call into this library.

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
| Code knowledge graph (callers, tests) | `repo_map::GraphBuilder`, `repo_map::Store` |
| Git operations and worktree management | `git::GitRepo`, `git::WorktreeManager` |
| Workspace settings and namespaces | `workspace::Workspace` |
| Enforce write boundaries per agent | `scope_enforcer` |
| Tree-sitter indentation heuristics | `indent::compute_indent()` |
| Embedded terminal (PTY) management | `terminal::TerminalManager` |

---

## Modules

| Module | Key Types / Functions |
|---|---|
| `types` | `FileScope`, `DiffHunk`, `WriteProposal`, `ModelTier`, `PrivacyLevel` |
| `iteration` | `IterationEngine::run()`, `Strategy`, `IterationConfig` |
| `validation_gate` | `ValidationGate` trait, `ValidationPipeline`, `TreeSitterGate`, `CargoCheckGate` |
| `scope_enforcer` | Path-level write boundary enforcement for agent file access |
| `repo_map` | `RepoMap`, `FileNode`, `ContextPlan` — PageRank-based context selection |
| `repo_map::graph_builder` | Build code knowledge graph (call edges, test associations) |
| `repo_map::store` | Persistent graph storage for cross-session reuse |
| `repo_map::edges` | Edge types: caller/callee, test ownership, import |
| `workspace` | `Workspace`, `WorkspaceFolder`, settings cascade |
| `session_state` | `SessionState`, `TabState`, `PanelState` |
| `tree_sitter` | `LANGUAGE_REGISTRY` (16 langs), `enrich_hunks()` |
| `diff_engine` | `compute_hunks()` |
| `observer` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` traits |
| `write_gate` | `WriteGatePipeline`, `WriteMode` |
| `acp` | `AcpSession`, `AcpPipeline`, `AcpSessionFactory` |
| `git` | `GitRepo`, `WorktreeManager` |
| `indent` | `compute_indent()`, `IndentResult`, tree-sitter + hybrid strategies |
| `query_loader` | Tree-sitter `.scm` file discovery |
| `memory` | `MemoryStore`, `OnnxEmbedder`, `CodeGraph`, `Consolidator` |
| `swarm` | `execute()`, `plan_coordinated()`, `Coordinator`, `TierRouter`, `PrivacyScanner` |
| `terminal` | `TerminalManager`, `TerminalInstance`, OSC 133 parsing |

---

## Usage

### Running a single-agent task

```rust
use gaviero_core::swarm::models::WorkUnit;
use gaviero_core::types::FileScope;

let unit = WorkUnit {
    id: "my-task".into(),
    description: "Add a hello function to src/lib.rs".into(),
    scope: FileScope {
        owned_paths: vec!["src/".into()],
        read_only_paths: vec![],
        interface_contracts: std::collections::HashMap::new(),
    },
    model: Some("sonnet".into()),
    max_retries: 3,
    // Graph-context fields (new):
    impact_scope: false,             // do not auto-expand read_only with blast-radius files
    context_callers_of: vec![],      // no caller-graph queries
    context_tests_for: vec![],       // no test-file queries
    context_depth: 2,                // BFS depth for graph traversal
    ..Default::default()
};
```

### Running a swarm from a compiled plan

```rust
use gaviero_core::swarm::pipeline::{execute, SwarmConfig};
use gaviero_core::swarm::plan::CompiledPlan;
use gaviero_core::observer::SwarmObserver;

let plan = CompiledPlan::from_work_units(vec![unit], None);

let config = SwarmConfig {
    workspace_root: std::path::PathBuf::from("."),
    model: "sonnet".into(),
    max_parallel: 1,
    use_worktrees: false,
    read_namespaces: vec!["my-project".into()],
    write_namespace: "my-project".into(),
    context_files: vec![],
};

struct MyObserver;
impl SwarmObserver for MyObserver { /* ... */ }

let result = execute(
    &plan, &config, None, None, &MyObserver,
    |id| Box::new(MyAcpObserver::new(id))
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

Gates available: `TreeSitterGate` (syntax), `CargoCheckGate` (compile), `CargoClippyGate` (lint), `CargoTestGate` (test suite), `ImpactTestGate` (tests for modified files only).

---

## Repo map and code knowledge graph

The repo map builds a PageRank-ranked file graph so agents receive the most relevant context first. The code knowledge graph extends this with explicit edge types:

```rust
use gaviero_core::repo_map::{RepoMap, GraphBuilder};

// Build context plan for a set of entry files
let repo_map = RepoMap::build(&workspace_root, &entry_files, config).await?;
let context_plan = repo_map.context_plan(token_budget);

// Query callers of a specific file
let callers = graph_store.callers_of("src/auth/session.rs", depth: 2)?;

// Find test files associated with a path
let tests = graph_store.tests_for("src/auth/", depth: 2)?;
```

These queries are exposed in the DSL via the `context {}` block inside `agent` declarations, and as `impact_scope true` in `scope {}` blocks.

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
- `SwarmObserver` — swarm-level events (phase changes, agent status, cost updates, tier dispatch)

Observers are passed as `&dyn Trait` to `execute()` and `run_backend()`, enabling zero-copy event routing with no lock contention.

---

## Supported languages (tree-sitter)

Rust, JavaScript, TypeScript, Python, Java, Kotlin, Bash, C, C++, HTML, CSS, JSON, YAML, TOML, LaTeX.
