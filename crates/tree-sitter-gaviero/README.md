# tree-sitter-gaviero

## Overview

Tree-sitter grammar and Rust binding for the `.gaviero` workflow language. Produces an incremental parse tree for syntax highlighting and structural queries. Semantic compilation (name resolution, scope checks, `CompiledPlan` generation) lives in [`gaviero-dsl`](../gaviero-dsl/README.md).

## Installation

```bash
cargo build -p tree-sitter-gaviero
cargo test  -p tree-sitter-gaviero
```

Generated C parser (`src/parser.c`, `src/grammar.json`, `src/node-types.json`) is committed — downstream crates build without the tree-sitter CLI.

## Usage

```rust
let mut parser = tree_sitter::Parser::new();
let language: tree_sitter::Language = tree_sitter_gaviero::LANGUAGE.into();
parser.set_language(&language)?;

let tree = parser.parse(
    r#"workflow demo { steps [agent_a] verify { compile true } }"#,
    None,
);
```

Inside the Gaviero workspace, use re-exports from `gaviero-core` instead of depending on `tree-sitter` directly:

```rust
use gaviero_core::tree_sitter::{Language, Parser, Query};
```

## Examples

Grammar surface includes `client`, `agent`, `workflow` declarations; `scope`, `memory`, `context`, `verify` blocks; explicit `loop {}` and `until` clauses; `include` directives; strings and comments.

The grammar accepts anything that should produce a meaningful downstream diagnostic — semantic errors are `gaviero-dsl`'s job.

| Need | Crate |
|---|---|
| Syntax tree / incremental parsing | `tree-sitter-gaviero` (this crate) |
| Semantic compilation | [`gaviero-dsl`](../gaviero-dsl/README.md) |
| Language registry (16 langs) | `gaviero-core::tree_sitter` |
| Editor highlighting | [`gaviero-tui`](../gaviero-tui/README.md) |

## Configuration

No runtime configuration. To update the grammar:

1. Edit `grammar.js` (single source of truth).
2. Run `npm run build` (requires tree-sitter CLI) to regenerate `parser.c`, `grammar.json`, `node-types.json`.
3. Update integration tests in `src/lib.rs`.
4. Commit `grammar.js` and all generated artefacts together.

**Never hand-edit `parser.c` or `grammar.json`.** Node shapes are part of the public contract with highlight queries in [`queries/gaviero/`](../../queries/gaviero).

## API

Exports a single public symbol:

```rust
pub const LANGUAGE: tree_sitter::Language;
```

Use via `tree_sitter_gaviero::LANGUAGE` or the `gaviero-core` re-export. No other public API surface.

## See Also

- [gaviero-dsl](../gaviero-dsl/README.md) — semantic compiler
- [Root README](../../README.md) — workflow DSL overview

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
