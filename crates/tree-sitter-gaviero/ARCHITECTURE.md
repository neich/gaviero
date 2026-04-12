# tree-sitter-gaviero - Architecture

This crate packages the tree-sitter grammar for `.gaviero` and exposes the Rust
`LANGUAGE` binding used by the workspace.

## File layout

```text
tree-sitter-gaviero/
├── grammar.js            grammar source of truth
├── tree-sitter.json      grammar metadata and binding config
├── build.rs              compiles generated C sources
├── src/
│   ├── lib.rs            Rust binding exposing `LANGUAGE`
│   ├── parser.c          generated parser
│   ├── grammar.json      generated grammar snapshot
│   └── node-types.json   generated node metadata
└── Cargo.toml
```

## Responsibilities

### `grammar.js`

Defines the syntax tree shape:

- declaration nodes for `client`, `agent`, and `workflow`
- block nodes for `scope`, `memory`, `context`, `verify`, and `loop`
- list, string, numeric, boolean, and identifier rules

This file is the hand-maintained grammar source.

### Generated parser artifacts

`parser.c`, `grammar.json`, and `node-types.json` are generated from the grammar
and committed so the Rust crate can build without re-running tree-sitter during
normal Cargo compilation.

### `src/lib.rs`

Exports the `LANGUAGE` constant and holds the crate-level tests that verify the
grammar can parse representative `.gaviero` snippets.

## Boundary with `gaviero-dsl`

`tree-sitter-gaviero` is intentionally syntax-focused.

- It parses source into a tolerant incremental syntax tree
- It does not resolve names, detect semantic errors, or build execution plans

Those semantic steps live in `crates/gaviero-dsl`, which uses its own lexer and
parser pipeline for compilation.

## Design intent

- Provide fast editor-friendly parsing for `.gaviero`
- Keep the grammar small and direct
- Separate syntax tooling from the semantic compiler
