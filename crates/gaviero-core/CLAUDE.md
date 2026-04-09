# gaviero-core

Core library for Gaviero. All pipeline logic lives here. No UI dependencies.

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
```

Some tests require network (Ollama health checks, model downloads). These are `#[ignore]` by default.

## Module Overview

| Module | Purpose |
|---|---|
| `types` | `FileScope`, `DiffHunk`, `WriteProposal`, `ModelTier`, `PrivacyLevel` |
| `workspace` | `Workspace`, `WorkspaceFolder`, settings cascade |
| `session_state` | `SessionState`, `TabState`, `PanelState` |
| `tree_sitter` | `LANGUAGE_REGISTRY` (16 langs), `enrich_hunks()`, language detection |
| `diff_engine` | `compute_hunks()` via `similar` crate |
| `write_gate` | `WriteGatePipeline`, `WriteMode` (Interactive/AutoAccept/RejectAll) |
| `observer` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` trait definitions |
| `git` | `GitRepo` (git2 wrapper), `WorktreeManager` |
| `query_loader` | Tree-sitter `.scm` query file discovery |
| `acp/` | `AcpSession` (Claude subprocess), `AcpPipeline` (prompt enrichment + file block routing), `AcpSessionFactory` |
| `swarm/` | Orchestration engine — see below |
| `memory/` | `MemoryStore` (SQLite + vector search), `OnnxEmbedder`, `CodeGraph`, `Consolidator` |
| `indent/` | `compute_indent()` — tree-sitter + hybrid + bracket strategies |
| `terminal/` | `TerminalManager`, PTY handling, OSC 133 parsing |
| `iteration/` | `IterationEngine` — retry loops with verification |
| `validation_gate/` | `ValidationGate` trait, `TreeSitterGate`, `CargoCheckGate` |
| `scope_enforcer/` | Path-level write boundary enforcement |
| `repo_map/` | `RepoMap`, PageRank-based context ranking |

## Swarm Subsystem (`swarm/`)

| File | Purpose |
|---|---|
| `models.rs` | `WorkUnit`, `AgentManifest`, `SwarmResult`, `MergeResult` |
| `pipeline.rs` | Tier orchestration, parallel execution, merge, loop conditions, escalation |
| `coordinator.rs` | Natural language → DAG planning, continuity detection |
| `planner.rs` | Task decomposition into `WorkUnit` list |
| `validation.rs` | Scope overlap detection, Kahn's topological sort |
| `router.rs` | `TierRouter` — model tier resolution (local/cheap/expensive) |
| `privacy.rs` | `PrivacyScanner` — glob-based privacy classification |
| `calibration.rs` | Tier performance stats, history queries |
| `plan.rs` | `CompiledPlan`, `PlanNode`, DAG with dependency edges |
| `execution_state.rs` | Checkpoint/resume for interrupted runs |
| `backend/` | `AgentBackend` trait, `ClaudeCodeBackend`, `OllamaStreamBackend`, `MockBackend` |
| `verify/` | `structural` (tree-sitter), `diff_review` (LLM), `test_runner`, `combined` |
| `merge.rs` | Git merge + Claude-powered conflict resolution |
| `bus.rs` | Inter-agent broadcast + targeted messaging |
| `board.rs` | Shared discovery board between agents |
| `context.rs` | Repository context collection for prompts |
| `replanner.rs` | Mid-execution replan decisions |

## Key Dependencies

- `tree-sitter 0.25` + 16 language grammars
- `git2 0.19` — all git operations
- `rusqlite 0.32` (bundled) + `sqlite-vec` — memory storage
- `ort 2.0.0-rc.12` + `tokenizers` — ONNX embedding inference
- `petgraph 0.8` — DAG operations
- `portable-pty` + `vt100` — terminal emulation

## Conventions

- Lock discipline: never hold Mutex across I/O, parsing, or embedding. Brief HashMap ops only.
- `AgentBackend` trait is object-safe for dynamic dispatch.
- All verification steps implement `run_verification()` in `verify/combined.rs`.
- Memory default model: e5-small-v2 (384 dimensions), brute-force cosine similarity.

See [ARCHITECTURE.md](../../ARCHITECTURE.md) for data flow diagrams.
