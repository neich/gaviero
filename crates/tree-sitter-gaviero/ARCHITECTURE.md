# tree-sitter-gaviero — Architecture

Tree-sitter grammar for `.gaviero` files. Produces the incremental syntax tree used by the editor; semantic analysis lives in [`gaviero-dsl`](../gaviero-dsl).

---

## 1. Topology

```
grammar.js  ──── tree-sitter CLI (npm run build) ───► src/parser.c
   │                                                  src/grammar.json
   │                                                  src/node-types.json
   │                                                            │
   │                                                            ▼
   └──────────►  build.rs ── cc ──►  libtree_sitter_gaviero  ◄──┘
                                              │
                                              ▼
                                  src/lib.rs::LANGUAGE
                                              │
                                              ▼
                       gaviero-core re-exports tree-sitter types
                                              │
                                              ▼
                       gaviero-tui editor highlight, indent, queries/
                       gaviero-dsl (independent lexer + parser — does
                                    not depend on this grammar at all)
```

The Rust crate compiles the generated C parser into a static library; downstream consumers reach this grammar through [`gaviero_core::Language`](../gaviero-core/src/lib.rs) re-exports.

---

## 2. File Layout

```
tree-sitter-gaviero/
├─ grammar.js            Hand-maintained grammar (single source of truth)
├─ tree-sitter.json      Binding metadata
├─ build.rs              Compiles src/parser.c via cc
├─ package.json          tree-sitter CLI dev install
├─ package-lock.json
├─ node_modules/         tree-sitter CLI (dev only)
├─ src/
│  ├─ lib.rs             Rust binding exposing LANGUAGE + integration tests
│  ├─ parser.c           Generated parser (committed)
│  ├─ grammar.json       Generated grammar snapshot (committed)
│  ├─ node-types.json    Generated node metadata (committed)
│  └─ tree_sitter/       tree-sitter headers (vendored)
└─ test/
   └─ corpus/            Tree-sitter corpus tests
```

---

## 3. Modules / Responsibilities

### [`grammar.js`](grammar.js) — syntax surface

Defines the parser:

- Top-level declarations: `client`, `agent`, `workflow`, `prompt`, `vars`, `tier`, `include`.
- Block nodes: `scope`, `memory`, `context`, `verify`, `loop`.
- Loop sub-rules: `agents`, `max_iterations`, `iter_start`, `stability`, `judge_timeout`, `strict_judge`, `branch_chain`, `until`.
- Literals: double-quoted strings, raw blocks (`#"..."#`), lists (`[a b c]`), integers, booleans, identifiers, path globs.

**Hand-maintained.** Edits go here; the generated artefacts are derived.

### Generated artefacts — `src/parser.c`, `src/grammar.json`, `src/node-types.json`

Produced by the tree-sitter CLI (`npm run build`). Committed so `cargo build` runs without requiring the tree-sitter CLI on the build machine.

### [`src/lib.rs`](src/lib.rs) — Rust binding

```rust
pub const LANGUAGE: tree_sitter_language::LanguageFn = …;

#[cfg(test)]
mod tests { /* corpus smoke tests */ }
```

[`build.rs`](build.rs) compiles `src/parser.c` via the `cc` crate; the binding plugs the resulting language into tree-sitter via `tree-sitter-language`.

---

## 4. Boundary with `gaviero-dsl`

This crate is **syntax-focused**. It builds a tolerant incremental syntax tree; it does **not**:

- resolve names (clients, agents, workflows, prompts, tiers, includes),
- detect scope-overlap or dependency-cycle errors,
- compile to [`CompiledPlan`](../gaviero-core/src/swarm/plan.rs),
- enforce var-substitution rules.

All of that lives in [`gaviero-dsl`](../gaviero-dsl), which runs an independent `logos`+`chumsky` pipeline (so DSL compilation does not pull tree-sitter into hot paths). The two parsers share the same surface — they must agree on what is grammatical — but the tree-sitter grammar is permissive: anything that should produce a meaningful diagnostic later is *accepted* here so `gaviero-dsl` can flag it with a source span.

The editor uses both: tree-sitter drives highlighting and node queries; `gaviero-dsl` drives compile-time validation when the user invokes `/run` or `--script`.

---

## 5. Editor Queries

Highlight + structural queries live in the workspace [`queries/gaviero/`](../../queries/gaviero) directory (separate from this crate). They reference node shapes defined in `grammar.js`; renaming a node here breaks highlights — bump intentionally.

---

## 6. Grammar Update Workflow

1. Edit [`grammar.js`](grammar.js) with new rules / nodes.
2. Regenerate: `npm run build` (uses the tree-sitter CLI in [`node_modules/`](node_modules)).
3. Commit `grammar.js`, `parser.c`, `grammar.json`, `node-types.json` together.
4. Update tests in [`src/lib.rs`](src/lib.rs) and the corpus under [`test/corpus/`](test/corpus) to cover new syntax.
5. CI validates that the parser compiles and tests pass.

---

## 7. Public API

```rust
pub const LANGUAGE: tree_sitter_language::LanguageFn;
```

That's it — downstream code uses [`gaviero_core::Language`](../gaviero-core/src/lib.rs) and the re-exported tree-sitter API rather than depending on `tree-sitter` directly.

---

## 8. Concurrency

None. Parser state is owned by the caller (a `tree_sitter::Parser`); this crate ships a single language pointer plus the compiled parser binary.

---

## 9. Error Handling

Tree-sitter is **error-tolerant by design** — the parser inserts `ERROR` nodes rather than failing. Downstream code (editor highlights, `gaviero-dsl`) inspects the tree for `ERROR` / `MISSING` nodes when surfacing diagnostics.

---

## 10. Design Intent

- Provide fast editor-friendly parsing for `.gaviero`.
- Keep the grammar small and direct.
- Separate syntax tooling from the semantic compiler — every meaningful diagnostic comes from [`gaviero-dsl`](../gaviero-dsl).
- Never import `tree-sitter` directly in downstream crates; always use [`gaviero_core`](../gaviero-core) re-exports so a single tree-sitter version threads the workspace.

---

## 11. Dependencies

- `tree-sitter-language 0.1` — Rust binding glue.
- `cc 1` — build script, compiles `parser.c`.
- `tree-sitter 0.25` — dev-only, integration tests.

---

See [CLAUDE.md](CLAUDE.md) for build commands and the grammar-update workflow.
