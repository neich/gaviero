# tree-sitter-gaviero

Tree-sitter grammar for `.gaviero` files. Generates incremental syntax tree for editor integration.

## Build & Test

```bash
cargo test -p tree-sitter-gaviero
cargo clippy -p tree-sitter-gaviero
```

Crate-level tests in `src/lib.rs` verify grammar against representative `.gaviero` snippets.

## File Layout

- `grammar.js` — hand-maintained grammar source (single source of truth)
- `tree-sitter.json` — grammar metadata and binding config
- `build.rs` — compiles generated C parser
- `src/lib.rs` — Rust bindings, exports `LANGUAGE` constant, integration tests
- `parser.c`, `grammar.json`, `node-types.json` — generated from grammar, committed to repo

## Key Rules

- **Edit grammar.js only.** Parser artifacts are generated.
- Never import `tree-sitter` crate downstream. Always use re-export from `gaviero-core::lib.rs`.
- Grammar scope: syntax only. Validation, name resolution, semantic analysis belong in `gaviero-dsl`.

## Grammar Updates

1. Edit `grammar.js` with new rules/nodes
2. Regenerate parser: `npm run build` (tree-sitter CLI)
3. Commit `grammar.js`, `parser.c`, `grammar.json`, `node-types.json`
4. Update crate tests in `src/lib.rs` to cover new syntax
5. CI validates parser compiles and tests pass

## What Goes Where

| Task | File |
|---|---|
| Add new token, fix operator precedence, change node shape | `grammar.js` |
| Validate `scope {}` overlaps, detect semantic errors | `gaviero-dsl/compiler.rs` |
| Parse agent/workflow declarations | Grammar (syntax tree structure) |
| Typecheck, resolve names, build `CompiledPlan` | `gaviero-dsl` compiler |

## Dependencies

- `tree-sitter 0.25` (C library)
- `cc` (build script, compiles parser.c)

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — grammar design, boundary with gaviero-dsl, syntax tree shape.