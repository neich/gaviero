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
| `ast.rs` | AST types: `Script`, `Item`, `AgentDecl`, `WorkflowDecl`, `PromptDecl`, `TierAlias`, blocks. **Authoritative** for DSL surface — read here before extending docs. |
| `parser.rs` | Chumsky parser → AST |
| `compiler.rs` | Semantic analysis → `CompiledPlan` (scope-overlap + dependency-cycle checks) |
| `error.rs` | Miette diagnostics with source spans |

## Public API

```rust
compile(source, filename, workflow, runtime_prompt) -> Result<CompiledPlan>
compile_with_vars(source, filename, workflow, runtime_prompt, override_vars, override_tiers) -> Result<CompiledPlan>
compile_file(entry_path, workflow, runtime_prompt, override_vars, override_tiers) -> Result<CompiledPlan>
load_tier_overrides(path) -> Result<Vec<(String, String)>>
```

`compile_with_vars` backs `gaviero-cli --var KEY=VALUE`. Var precedence: agent-level `vars {}` > CLI `--var` overrides > script-level `vars {}`.

`load_tier_overrides` + `override_tiers` backs `gaviero-cli --tiers-file <profile.gaviero>`. Tier precedence: CLI `--tiers-file` > script/includes `tier` lines.

## DSL Surface (high-level)

- Top-level items: `client`, `agent`, `workflow`, `prompt`, `vars`, `tier <alias> <client-ref>`.
- `prompt <name> #" ... "#` — reusable named prompts; substitutes `{{AGENT}}`, `{{PROMPT}}`, and in-scope vars.
- `vars {}` — compile-time substitution across prompts, descriptions, memory content, and scope paths.
- `tier <alias> <client-ref>` — re-pointable routing label so agents say `tier expensive` instead of binding to a specific client.
- `client {}` fields: `tier`, `model`, `privacy`, `effort`, `default`, `extra { "k" "v" }` (provider-specific pass-through).
- Scope glob patterns enforced via `gaviero_core::path_pattern::paths_overlap`. Disjoint globs allowed; overlap within the same `loop { agents [...] }` group is allowed.
- Workflow blocks: `steps`, `depends_on`, `max_parallel`, `loop { until agent <judge> ... }` with `stability`, `iter_start`, `max_iterations`, `judge_timeout`, `strict_judge`.

For exact field names and shapes, read `ast.rs`.

## Conventions

- Errors carry source spans. Always propagate span info.
- Compiler validates scope overlaps and dependency cycles at compile time. Never bypass these checks.
- Provider-neutral model strings — resolved at runtime by `gaviero-core`.
- `vars` substitution is single-pass: `{{FOO}}` expands; nested refs inside values do not.

## Dependencies

- `gaviero-core` — `CompiledPlan`, `path_pattern`, types
- `logos`, `chumsky` — lexing/parsing
- `miette`, `thiserror` — diagnostics

## See Also

- [README.md](README.md) — language reference, examples
- [ARCHITECTURE.md](ARCHITECTURE.md) — compilation pipeline, output types, scope validation
