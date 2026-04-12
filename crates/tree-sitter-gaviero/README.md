# tree-sitter-gaviero

`tree-sitter-gaviero` provides the tree-sitter grammar and Rust binding for the
`.gaviero` workflow language.

This crate is syntax tooling, not the semantic compiler. The real DSL compiler
lives in `crates/gaviero-dsl`. Use this crate when you need incremental parsing,
syntax highlighting, editor integration, or tree-sitter-based structural checks.

## What it parses

The grammar covers the main `.gaviero` syntax:

- `client`, `agent`, and `workflow` declarations
- `scope`, `memory`, `context`, and `verify` blocks
- explicit `loop {}` blocks and `until` clauses
- quoted strings, raw strings, identifiers, integers, floats, and comments

## Rust usage

```rust
let mut parser = tree_sitter::Parser::new();
let language: tree_sitter::Language = tree_sitter_gaviero::LANGUAGE.into();
parser.set_language(&language)?;

let tree = parser.parse(
    r#"workflow demo { steps [agent_a] verify { compile true } }"#,
    None,
);
```

## Relationship to the rest of the workspace

- `gaviero-dsl`: semantic compilation and diagnostics
- `gaviero-tui`: editor/highlighting integration
- `gaviero-core`: tree-sitter-based editor/runtime helpers

If you need execution semantics, provider routing, or workflow compilation, this
is not the right crate.
