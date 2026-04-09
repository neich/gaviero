# gaviero-dsl

Compiler for `.gaviero` workflow scripts. Transforms DSL source into `CompiledPlan` DAGs consumed by the swarm pipeline.

## Build & Test

```bash
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Pipeline

```
Source (.gaviero) → Lexer (logos) → Parser (chumsky) → AST → Compiler → CompiledPlan
```

| Module | Purpose |
|---|---|
| `lexer.rs` | `lex()` — logos-based tokenizer |
| `ast.rs` | `Script`, `AgentDecl`, `WorkflowDecl`, `ContextBlock`, `ScopeBlock`, `MemoryBlock`, `VerifyBlock` |
| `parser.rs` | `parse()` — chumsky-based parser → AST |
| `compiler.rs` | `compile_ast()` → `CompiledPlan` (DAG of `PlanNode`s with `WorkUnit`s) |
| `error.rs` | `DslError`, `DslErrors` — miette diagnostic errors with spans |

**Entry point:** `compile(source, filename, workflow, runtime_prompt) → Result<CompiledPlan>`

## Key DSL Features

- Agent declarations with `scope {}`, `memory {}`, `verify {}`, `context {}` blocks
- Workflow declarations with ordered agent references and `depends_on`
- `impact_scope` and `impact_tests` for verification configuration
- `context {}` block: `callers_of`, `tests_for`, `depth` for repo-map-driven context

## Dependencies

- `gaviero-core` — for `CompiledPlan`, `WorkUnit`, `PlanNode` types
- `logos` — lexer generator
- `chumsky` — parser combinator
- `miette` — error diagnostics with source spans
- `thiserror` — error type derivation

## Conventions

- Errors carry source spans for precise diagnostics. Always propagate span info.
- The compiler validates scope overlaps and dependency cycles at compile time.
- Example scripts live in `examples/`.
