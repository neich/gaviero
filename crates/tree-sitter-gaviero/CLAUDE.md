# tree-sitter-gaviero

Tree-sitter grammar for `.gaviero` files. Produces the incremental syntax tree used by the editor; **semantics belong to [`gaviero-dsl`](../gaviero-dsl/CLAUDE.md)**.

## Build & Test

```bash
cargo test -p tree-sitter-gaviero
cargo clippy -p tree-sitter-gaviero
```

Crate-level tests in [`src/lib.rs`](src/lib.rs) verify the grammar against representative `.gaviero` snippets.

## Architecture

- [`grammar.js`](grammar.js) — hand-maintained grammar (single source of truth).
- [`tree-sitter.json`](tree-sitter.json) — binding metadata.
- [`build.rs`](build.rs) — compiles the generated C parser via `cc`.
- [`src/lib.rs`](src/lib.rs) — Rust bindings; exports `LANGUAGE`.
- [`src/parser.c`](src/parser.c), [`src/grammar.json`](src/grammar.json), [`src/node-types.json`](src/node-types.json) — **generated**; committed so downstream builds without the tree-sitter CLI.

### Grammar update workflow

1. Edit [`grammar.js`](grammar.js).
2. Regenerate: `npm run build` (tree-sitter CLI in [`node_modules/`](node_modules)).
3. Commit `grammar.js` + generated files together.
4. Update tests in [`src/lib.rs`](src/lib.rs).
5. CI validates compile + tests.

### What goes where

| Task | Where |
|---|---|
| Token / precedence / node shape | [`grammar.js`](grammar.js) |
| Scope overlap, name resolution, `CompiledPlan` | [`gaviero-dsl`](../gaviero-dsl/CLAUDE.md) |
| Editor highlights | [`queries/gaviero/highlights.scm`](../../queries/gaviero/highlights.scm) |

## Conventions

- **Syntax only.** Scope overlap, name resolution, cycles → `gaviero-dsl`.
- Accept token shapes that semantic analysis can diagnose more usefully than a hard grammar reject.
- Node shapes are a public contract with [`queries/gaviero/`](../../queries/gaviero) — rename intentionally.

## Rules

- **Edit `grammar.js` only.** Never hand-edit `parser.c`, `grammar.json`, or `node-types.json`.
- **Never `use tree_sitter::*` in downstream crates.** Use [`gaviero_core`](../gaviero-core/src/lib.rs) re-exports (`Language`, `Parser`, `Node`, `Query`, `Tree`, …).
- **Do not embed semantic checks** — even tempting ones like `scope {}` overlap belong in [`gaviero-dsl/compiler.rs`](../gaviero-dsl/src/compiler.rs).

## Dependencies

- `tree-sitter-language 0.1` — Rust binding.
- `cc 1` — build script compiles `parser.c`.
- `tree-sitter 0.25` (dev) — integration tests.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — grammar design, boundary with `gaviero-dsl`.
- [README.md](README.md) — usage example, parsed-node scope.
