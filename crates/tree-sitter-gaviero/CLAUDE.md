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
- [`src/lib.rs`](src/lib.rs) — Rust bindings; exports `LANGUAGE` and runs integration tests.
- [`src/parser.c`](src/parser.c), [`src/grammar.json`](src/grammar.json), [`src/node-types.json`](src/node-types.json) — **generated** from `grammar.js`; committed to the repo so downstream crates build without the tree-sitter CLI.

## Grammar Update Workflow

1. Edit [`grammar.js`](grammar.js).
2. Regenerate: `npm run build` (tree-sitter CLI in [`node_modules/`](node_modules)).
3. Commit `grammar.js`, `parser.c`, `grammar.json`, `node-types.json` together.
4. Update tests in [`src/lib.rs`](src/lib.rs) to cover new syntax.
5. CI validates that the parser compiles and tests pass.

## Conventions

- **Syntax only.** Every validation (scope overlap, name resolution, type checks, dependency cycles) is `gaviero-dsl`'s job.
- The grammar must accept anything that should produce a meaningful diagnostic downstream — never reject a token shape that semantic analysis can flag more usefully.
- Node shapes are part of the public contract with the editor's tree-sitter queries ([`queries/gaviero/`](../../queries/gaviero)). Renaming a node breaks highlights — bump intentionally.

## Rules

- **Edit `grammar.js` only.** Never hand-edit `parser.c`, `grammar.json`, or `node-types.json`; they are regenerated.
- **Never `use tree_sitter::*` in downstream crates.** Always go through the [`gaviero_core`](../gaviero-core/src/lib.rs) re-exports (`Language`, `Parser`, `Node`, `Query`, `Tree`, …). This keeps a single tree-sitter version in the dependency graph.
- **Do not embed semantic checks here** — even tempting cases like `scope {}` overlap detection belong in [`gaviero-dsl/compiler.rs`](../gaviero-dsl/src/compiler.rs).

## What Goes Where

| Task | File |
|---|---|
| Add a new token, fix operator precedence, change node shape | [`grammar.js`](grammar.js) |
| Validate `scope {}` overlaps, detect semantic errors | [`gaviero-dsl/compiler.rs`](../gaviero-dsl/src/compiler.rs) |
| Parse agent/workflow declarations | [`grammar.js`](grammar.js) (syntax tree structure) |
| Typecheck, resolve names, build `CompiledPlan` | [`gaviero-dsl`](../gaviero-dsl/CLAUDE.md) |
| Syntax-highlight nodes in the editor | [`queries/gaviero/highlights.scm`](../../queries/gaviero/highlights.scm) |

## Dependencies

- `tree-sitter-language 0.1` (Rust binding).
- `cc 1` (build script — compiles `parser.c`).
- `tree-sitter 0.25` (dev only — integration tests).

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) — grammar design, boundary with `gaviero-dsl`, syntax-tree shape.
- [README.md](README.md) — usage example, parsed-node scope.
