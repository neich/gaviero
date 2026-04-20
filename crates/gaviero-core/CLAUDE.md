# gaviero-core

All pipeline logic: swarm, memory, ACP (Claude subprocess), write gate, git, terminal, validation. No UI dependencies.

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
```

Network tests (Ollama, model downloads) are `#[ignore]` by default.

## Public Modules (from `src/lib.rs`)

| Module | Purpose |
|---|---|
| `swarm` | Multi-agent orchestration, tier routing, verification, merge |
| `memory` | 5-level scoped ONNX embeddings + consolidation |
| `acp` | Claude subprocess sessions (persistent / one-shot) |
| `write_gate` | File modification boundary + diff review |
| `validation_gate` | Structural + semantic validation pipeline |
| `scope_enforcer` | `FileScope` enforcement on proposals |
| `path_pattern` | Glob-aware scope overlap detection (backs DSL scope validation) |
| `agent_session` | Per-agent session lifetime + state plumbing |
| `context_planner` | Pre-prompt context assembly (graph + memory + files) |
| `session_state` | Checkpointable session state for resume |
| `repo_map` | Code knowledge graph (tree-sitter driven) |
| `tree_sitter` | 16-language registry, query loader |
| `diff_engine` | Unified diff generation / application |
| `git` | `git2` wrapper, worktrees, branches |
| `terminal` | PTY + OSC 133 + `vt100` emulation |
| `indent` | Hybrid indent detection |
| `iteration` | Iteration/retry loop control |
| `observer` | `WriteGateObserver`, `AcpObserver`, `SwarmObserver` traits |
| `query_loader` | Tree-sitter query discovery |
| `types` | Shared boundary types |
| `workspace` | `.gaviero/settings.json` + workspace discovery |

## Key Subsystems

**Swarm** (`swarm/`): tier execution (local/cheap/expensive), parallel fan-out, verification gates, scope validation, dependency DAG, checkpoint/resume, conflict resolution.

**Memory** (`memory/`): SQLite + sqlite-vec, nomic-embed-text-v1.5 ONNX. Cascading scope search (global → workspace → repo → module → run), early-terminate at 0.70 confidence. 3-phase consolidation: triage → decay/prune → cross-scope promotion.

**ACP** (`acp/`): Claude subprocess — session factory, argv/tempfile prompt spill, streaming file-block extraction.

**Write Gate** (`write_gate/`): interactive / auto-accept / reject-all modes; diff review; scope validation; observer callbacks.

## Conventions

- Lock discipline: never hold Mutex across I/O, parsing, or embedding.
- `AgentBackend` trait is object-safe; all backends in `swarm/backend/` implement it.
- Embedding model: nomic-embed-text-v1.5, cosine similarity.
- Memory writes require explicit `WriteScope` — never infer.
- Scoring: 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights.
- Hybrid search: RRF merges vector (0.7) + FTS (0.3).

## Dependencies

- `tree-sitter 0.25` + 16 grammars
- `git2 0.19`
- `rusqlite 0.32` (bundled) + `sqlite-vec`
- `ort 2.0.0-rc.12` + `tokenizers` (ONNX inference)
- `petgraph 0.8` (DAG ops)
- `portable-pty` + `vt100` (terminal emulation)

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — module map, swarm/memory pipelines, data flow diagrams.
