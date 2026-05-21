# tree-sitter-gaviero

`tree-sitter-gaviero` provides the tree-sitter grammar and Rust binding for the
`.gaviero` workflow language.

This crate is syntax tooling, not the semantic compiler. The real DSL compiler
lives in `crates/gaviero-dsl`. Use this crate when you need incremental parsing,
syntax highlighting, editor integration, or tree-sitter-based structural checks.

## Installation & Build

```bash
cargo build -p tree-sitter-gaviero
cargo test -p tree-sitter-gaviero
```

To regenerate the parser after editing `grammar.js`:

```bash
npm run build    # requires the tree-sitter CLI
```

## What it parses

The grammar covers the main `.gaviero` syntax:

- `client`, `agent`, and `workflow` declarations
- `scope`, `memory`, `context`, and `verify` blocks
- explicit `loop {}` blocks and `until` clauses
- `include` directives
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

Access this crate through `gaviero-core`'s re-exports, not directly:

```rust
// Correct — use the re-export
use gaviero_core::tree_sitter::{Language, Parser, Query};

// Avoid — never import the tree-sitter crate directly in downstream code
// use tree_sitter::Parser;
```

## Relationship to the rest of the workspace

| Need | Crate |
|---|---|
| Syntax tree / incremental parsing | `tree-sitter-gaviero` (this crate) |
| Semantic compilation, name resolution, `CompiledPlan` | `gaviero-dsl` |
| Tree-sitter language registry (16 languages) | `gaviero-core::tree_sitter` |
| Syntax highlighting in the editor | `gaviero-tui` |

If you need execution semantics, provider routing, or workflow compilation, this
is not the right crate.

## Grammar updates

1. Edit `grammar.js` — this is the single source of truth
2. Run `npm run build` to regenerate `parser.c`, `grammar.json`, `node-types.json`
3. Update integration tests in `src/lib.rs`
4. Commit all generated artefacts alongside `grammar.js`

Never edit `parser.c` or `grammar.json` directly.
