# gaviero-dsl

Compiler for `.gaviero` workflow scripts. Lexer (logos) → Parser (chumsky) → AST → Compiler → `CompiledPlan` DAG.

## Build & Test

```bash
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Modules

| File | Purpose |
|---|---|
| `lexer.rs` | Logos tokenizer |
| `ast.rs` | AST types: `Script`, `AgentDecl`, `WorkflowDecl`, blocks |
| `parser.rs` | Chumsky parser → AST |
| `compiler.rs` | Semantic analysis: `compile_ast()` → `CompiledPlan` |
| `error.rs` | Miette diagnostics with source spans |

Entry point: `compile(source, filename, workflow, runtime_prompt) → Result<CompiledPlan>`

## DSL Features

- Agent declarations: `scope {}`, `memory {}`, `verify {}`, `context {}` blocks
- Workflows with ordered agents, `depends_on` edges
- Context selection: `callers_of`, `tests_for`, `depth` (repo-map-driven)
- Verification config: `impact_scope`, `impact_tests`

## Conventions

- Errors carry source spans. Always propagate span info.
- Compiler validates scope overlaps and cycles at compile time.
- Provider-neutral model strings (resolved at runtime by gaviero-core).

## Dependencies

- `gaviero-core` — types
- `logos`, `chumsky` — lexing/parsing
- `miette`, `thiserror` — diagnostics

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline, output types, scope validation.
