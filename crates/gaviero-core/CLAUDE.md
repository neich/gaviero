# gaviero-core

All pipeline logic: swarm, memory, ACP (Claude subprocess), write gate, git, terminal, validation.

## Build & Test

```bash
cargo test -p gaviero-core
cargo clippy -p gaviero-core
```

Network tests (`Ollama`, model downloads) are `#[ignore]` by default.

## Key Subsystems

**Swarm** (`swarm/`): Multi-agent orchestration engine.
- Tier execution (local/cheap/expensive) with parallel fan-out
- Verification gates (structural, diff review, test runner)
- Git merge + Claude-powered conflict resolution
- Scope validation, dependency DAG, checkpoint/resume

**Memory** (`memory/`): 5-level scoped embeddings (global → workspace → repo → module → run).
- SQLite + sqlite-vec backend, ONNX embedder (nomic-embed-text-v1.5)
- Cascading search narrows scope, early-terminates at 0.70 confidence
- 3-phase consolidation: triage → decay/prune → cross-scope promotion

**ACP** (`acp/`): Claude subprocess integration.
- Session factory, persistent or one-shot modes
- Prompt enrichment, file block routing, streaming

**Write Gate** (`write_gate/`): File modification boundary enforcement.
- Interactive/auto-accept/reject-all modes
- Diff review, scope validation, observer callbacks

**Utilities**: git (git2 wrapper + worktrees), tree-sitter (16-lang registry, query loader), diff engine, terminal (PTY + OSC 133), indent (hybrid strategy).

## Conventions

- Lock discipline: never hold Mutex across I/O, parsing, or embedding.
- `AgentBackend` trait is object-safe; all backends in `backend/` implement it.
- Memory model: nomic-embed-text-v1.5, cosine similarity.
- Memory writes require explicit `WriteScope` — never infer.
- Scoring: 50% similarity + 20% importance + 15% recency + 15% base, scaled by scope/trust weights.
- Hybrid search: RRF merges vector (0.7) + FTS (0.3) results.

## Dependencies

- `tree-sitter 0.25` + 16 grammars
- `git2 0.19`
- `rusqlite 0.32` (bundled) + `sqlite-vec`
- `ort 2.0.0-rc.12` + `tokenizers` (ONNX inference)
- `petgraph 0.8` (DAG ops)
- `portable-pty` + `vt100` (terminal emulation)

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — module map, swarm/memory pipelines, data flow diagrams.
