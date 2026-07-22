# tree-sitter-gaviero — Architecture

Tree-sitter grammar for `.gaviero` files. Produces the incremental syntax tree used by the editor; semantic analysis lives in [`gaviero-dsl`](../gaviero-dsl).

Conventions / regenerate workflow: [CLAUDE.md](CLAUDE.md).

---

## Topology

```
grammar.js  ── npm run build ──► src/parser.c
                                 src/grammar.json
                                 src/node-types.json
        │                                │
        └──── build.rs (cc) ─────────────┘
                      │
                      ▼
              src/lib.rs::LANGUAGE
                      │
                      ▼
         gaviero-core re-exports tree-sitter types
                      │
          ┌───────────┴────────────┐
          ▼                        ▼
   gaviero-tui highlight     gaviero-dsl (independent
   + indent + queries/       logos+chumsky — no dep
                             on this crate)
```

---

## Modules

```
tree-sitter-gaviero/
├─ grammar.js           Hand-maintained source of truth
├─ build.rs             Compiles src/parser.c via cc
├─ package.json         tree-sitter CLI (dev)
├─ src/
│  ├─ lib.rs            LANGUAGE export + integration tests
│  ├─ parser.c          Generated (committed)
│  ├─ grammar.json      Generated (committed)
│  └─ node-types.json   Generated (committed)
└─ test/corpus/         Tree-sitter corpus tests
```

Editor highlight / indent queries live in workspace [`queries/gaviero/`](../../queries/gaviero) (not this crate). Renaming nodes in `grammar.js` breaks those queries.

---

## Abstractions

### `grammar.js`

Declares top-level `client` / `agent` / `workflow` / `prompt` / `vars` / `tier` / `include`, block nodes (`scope`, `memory`, `context`, `verify`, `loop`), loop sub-rules, and literals (quoted / raw strings, lists, ints, bools, path globs).

**Never edit** `parser.c` / `grammar.json` / `node-types.json` by hand — regenerate from `grammar.js`.

### `LANGUAGE` ([`src/lib.rs`](src/lib.rs))

```rust
pub const LANGUAGE: tree_sitter_language::LanguageFn;
```

Downstream code reaches the grammar through [`gaviero_core::Language`](../gaviero-core/src/lib.rs) re-exports — never `use tree_sitter` directly.

### Boundary with `gaviero-dsl`

This crate is syntax-only. It does not resolve names, check scope overlap, expand vars, or emit `CompiledPlan`. The tree-sitter grammar is intentionally permissive so `gaviero-dsl` can attach span diagnostics.

---

## Data Flow

```
Edit grammar.js → npm run build → commit generated artefacts together
cargo build → build.rs compiles parser.c → LANGUAGE linked into gaviero-core
Editor: Parser::set_language(LANGUAGE) → Query on queries/gaviero/*.scm
```

---

## Concurrency

None. Callers own `tree_sitter::Parser` state. This crate ships a language function pointer + static library.

---

## Error Handling

Tree-sitter inserts `ERROR` / `MISSING` nodes rather than failing. Downstream (highlights, dsl) inspects the tree for diagnostics.

---

## API

```rust
pub const LANGUAGE: tree_sitter_language::LanguageFn;
```

Dependencies: `tree-sitter-language` (binding), `cc` (build), `tree-sitter 0.25` (dev tests only).
