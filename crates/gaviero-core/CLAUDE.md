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

**Memory** (`memory/`): SQLite + sqlite-vec, ONNX embedder via pluggable `Embedder` trait (default `gte-modernbert-base`; legacy `nomic-embed-text-v1.5` selectable). Multi-DB registry (global / workspace / per-folder). Single-consumer writer task owns all writes. Merged multi-scope retrieval with optional cross-encoder reranker; the legacy 0.70 cascading early-exit is retained behind `memory.retrieval.mode = "cascade"` as a kill-switch only. Three-cadence consolidation: per-turn extractor (S3) → per-session consolidator (B5) → idle/weekly sleeptime pass (B5: decay sweep, near-dup merge, cross-scope promotion, trust re-scoring, history compression, summary prune). Per-injection `injection_manifests` capture the full candidate pool. Soft-delete via `/forget` writes to a `deletions` audit table; History rows are immutable except via the C2.4 redaction path.

**ACP** (`acp/`): Claude subprocess — session factory, argv/tempfile prompt spill, streaming file-block extraction.

**Write Gate** (`write_gate/`): interactive / auto-accept / reject-all modes; diff review; scope validation; observer callbacks.

## Conventions

- Lock discipline: never hold Mutex across I/O, parsing, or embedding.
- `AgentBackend` trait is object-safe; all backends in `swarm/backend/` implement it.
- Embedder is pluggable (`Embedder` trait); default `gte-modernbert-base` (768 dim). Cosine similarity.
- Memory writes require explicit `WriteScope` — never infer. All writes flow through the writer task (`WriterMessage` mpsc).
- Scoring: 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights. B4 recency floor + decay-exempt types (Decision/Convention/Invariant/Preference) protect reference memories from age-out.
- Hybrid search: RRF merges vector (0.7) + FTS (0.3); merged multi-scope retrieval (B3) is the default.
- Optional cross-encoder reranker (B2) blends with composite score; off by default until eval gates green.

## Dependencies

- `tree-sitter 0.25` + 16 grammars
- `git2 0.19`
- `rusqlite 0.32` (bundled) + `sqlite-vec`
- `ort 2.0.0-rc.12` + `tokenizers` (ONNX inference)
- `petgraph 0.8` (DAG ops)
- `portable-pty` + `vt100` (terminal emulation)

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — module map, swarm/memory pipelines, data flow diagrams.
