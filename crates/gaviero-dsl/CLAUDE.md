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
| `ast.rs` | AST types: `Script`, `Item`, `AgentDecl`, `WorkflowDecl`, `PromptDecl`, `TierAlias`, blocks |
| `parser.rs` | Chumsky parser → AST |
| `compiler.rs` | Semantic analysis: `compile_ast_with_vars()` → `CompiledPlan` |
| `error.rs` | Miette diagnostics with source spans |

## Public API

```rust
compile(source, filename, workflow, runtime_prompt) -> Result<CompiledPlan>
compile_with_vars(source, filename, workflow, runtime_prompt, override_vars) -> Result<CompiledPlan>
```

`compile_with_vars` backs `gaviero-cli --var KEY=VALUE`. Overrides beat script-level `vars {}` but lose to agent-level `vars {}`.

## DSL Features

- **Top-level items**: `client`, `agent`, `workflow`, `prompt`, `vars`, `tier <alias> <client-ref>`.
- **`prompt <name> #" ... "#`**: reusable named prompts referenced by agents via `prompt some-name`. Substitutes `{{AGENT}}`, `{{PROMPT}}`, and any in-scope vars.
- **`vars { KEY "value" ... }`**: compile-time substitution applied across agent prompts, descriptions, memory content, AND scope paths (owned / read_only / staleness_sources).
- **`tier <alias> <client-ref>`**: routing label so agents can say `tier expensive` instead of binding to a specific client — re-pointable at launch.
- **`client`**: `tier`, `model`, `privacy`, `effort` (off/auto/low/medium/high/xhigh/max), `default`, and `extra { "k" "v" }` pass-through for provider-specific keys.
- **Scope glob patterns**: `owned ["plans/claude-*.md"]` and `owned ["plans/codex-*.md"]` are disjoint; overlap is computed by `gaviero_core::path_pattern::paths_overlap`. Overlap within the same `loop { agents [...] }` group is allowed.
- **Blocks**: `scope {}`, `memory {}`, `verify {}`, `context {}`, `loop {}`.
- **Workflows**: ordered `steps`, `depends_on`, `max_parallel`, loop `until agent <judge>` with `stability`, `iter_start`, `max_iterations`, `judge_timeout`, `strict_judge`.
- **Context selection**: `callers_of`, `tests_for`, `depth` (repo-map-driven).
- **Verification**: `impact_scope`, `impact_tests`.

## Conventions

- Errors carry source spans. Always propagate span info.
- Compiler validates scope overlaps and dependency cycles at compile time.
- Provider-neutral model strings (resolved at runtime by `gaviero-core`).
- `vars` substitution is single-pass: `{{FOO}}` expands; nested refs inside values do not.

## Dependencies

- `gaviero-core` — `CompiledPlan`, `path_pattern`, types
- `logos`, `chumsky` — lexing/parsing
- `miette`, `thiserror` — diagnostics

## See Also

[ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline, output types, scope validation.
