# tree-sitter-gaviero

Tree-sitter grammar and Rust binding for the `.gaviero` workflow language.

## Overview

This crate is **syntax tooling** — it produces an incremental parse tree for `.gaviero` files. It is *not* the semantic compiler; name resolution, scope-overlap checks, tier resolution, and `CompiledPlan` generation all live in [`gaviero-dsl`](../gaviero-dsl/README.md).

Reach for this crate when you need incremental parsing, syntax highlighting, editor integration, or tree-sitter-based structural queries. Reach for `gaviero-dsl` when you need execution semantics, provider routing, or workflow compilation.

## Installation

```bash
cargo build -p tree-sitter-gaviero
cargo test  -p tree-sitter-gaviero
```

The generated C parser (`src/parser.c`, `src/grammar.json`, `src/node-types.json`) is committed to the repo, so downstream crates build without the tree-sitter CLI. You only need the CLI to regenerate the parser after editing the grammar.

## Usage

The crate exports a single `LANGUAGE` constant:

```rust
let mut parser = tree_sitter::Parser::new();
let language: tree_sitter::Language = tree_sitter_gaviero::LANGUAGE.into();
parser.set_language(&language)?;

let tree = parser.parse(
    r#"workflow demo { steps [agent_a] verify { compile true } }"#,
    None,
);
```

Inside the Gaviero workspace, use the re-exports from `gaviero-core` instead of depending on the `tree-sitter` crate directly — this keeps a single tree-sitter version in the dependency graph:

```rust
// Correct — via the re-export
use gaviero_core::tree_sitter::{Language, Parser, Query};

// Avoid — never import the tree-sitter crate directly in downstream code
// use tree_sitter::Parser;
```

## What it parses

The grammar covers the `.gaviero` surface syntax:

- `client`, `agent`, and `workflow` declarations
- `scope`, `memory`, `context`, and `verify` blocks
- explicit `loop {}` blocks and `until` clauses
- `include` directives
- quoted strings, raw strings, identifiers, integers, floats, and comments

It intentionally accepts anything that should produce a meaningful downstream diagnostic — flagging semantic errors is `gaviero-dsl`'s job, not the grammar's.

## Where things live

| Need | Crate |
|---|---|
| Syntax tree / incremental parsing | `tree-sitter-gaviero` (this crate) |
| Semantic compilation, name resolution, `CompiledPlan` | [`gaviero-dsl`](../gaviero-dsl/README.md) |
| Tree-sitter language registry (16 languages) | `gaviero-core::tree_sitter` |
| Syntax highlighting in the editor | [`gaviero-tui`](../gaviero-tui/README.md) |

## Updating the grammar

1. Edit `grammar.js` — the single source of truth.
2. Run `npm run build` (requires the tree-sitter CLI) to regenerate `parser.c`, `grammar.json`, and `node-types.json`.
3. Update the integration tests in `src/lib.rs`.
4. Commit `grammar.js` and all generated artefacts together.

**Never hand-edit `parser.c` or `grammar.json`.** Node shapes are part of the public contract with the editor's highlight queries ([`queries/gaviero/`](../../queries/gaviero)); renaming a node breaks highlighting, so bump those intentionally.

## See Also

- [`crates/gaviero-dsl/README.md`](../gaviero-dsl/README.md) — the semantic compiler
- [Root README](../../README.md) — workflow DSL overview

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
