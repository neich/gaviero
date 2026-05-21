# gaviero-dsl

Compiler for `.gaviero` workflow scripts. Pipeline: include resolver → logos lexer → chumsky parser → AST → semantic analysis → `CompiledPlan` DAG.

## Build & Test

```bash
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Modules (from [src/lib.rs](src/lib.rs))

| File | Purpose |
|---|---|
| [`lexer.rs`](src/lexer.rs) | Logos tokenizer. |
| [`parser.rs`](src/parser.rs) | Chumsky combinators → AST. |
| [`ast.rs`](src/ast.rs) | AST types: `Script`, `Item`, `AgentDecl`, `WorkflowDecl`, `PromptDecl`, `TierAlias`, blocks. **Authoritative** for the DSL surface — read here before extending docs. |
| [`compiler.rs`](src/compiler.rs) | Semantic analysis → `CompiledPlan` (scope-overlap checks, dependency cycles, tier resolution, var substitution). |
| [`resolver.rs`](src/resolver.rs) | `include "..."` resolution: relative paths, cycle detection, idempotent dedup. Drives the `compile_file` entry point. |
| [`tiers.rs`](src/tiers.rs) | `--tiers-file` loader: parses a `.gaviero` that contains only `tier <alias> <client-ref>` lines and returns `Vec<(alias, client)>`. |
| [`error.rs`](src/error.rs) | `DslError` / `DslErrors` (miette diagnostics with source spans). |

## Public API

```rust
compile(source, filename, workflow, runtime_prompt) -> Result<CompiledPlan>
compile_with_vars(source, filename, workflow, runtime_prompt, override_vars, override_tiers)
compile_file(entry_path, workflow, runtime_prompt, override_vars, override_tiers)
load_tier_overrides(path) -> Result<Vec<(String, String)>>
```

- `compile_with_vars` backs `gaviero-cli --var KEY=VALUE`. **Var precedence:** agent-level `vars {}` > CLI `--var` overrides > script-level `vars {}`.
- `compile_file` is the file-path entry point — it runs the include resolver first, then lex/parse/compile. Use it whenever inputs come from disk. `compile` / `compile_with_vars` reject `include` statements with a diagnostic pointing here.
- `load_tier_overrides` + `override_tiers` backs `gaviero-cli --tiers-file <profile.gaviero>`. **Tier precedence:** CLI `--tiers-file` > script/includes `tier` lines.

## DSL Surface (high-level)

- Top-level items: `client`, `agent`, `workflow`, `prompt`, `vars`, `tier <alias> <client-ref>`, `include "path.gaviero"`.
- `prompt <name> #" ... "#` — reusable named prompts; substitutes `{{AGENT}}`, `{{PROMPT}}`, and in-scope `vars`.
- `vars {}` — compile-time substitution across prompts, descriptions, memory content, scope paths. Single-pass: `{{FOO}}` expands once; nested refs in values do not.
- `tier <alias> <client-ref>` — re-pointable routing label so agents say `tier expensive` instead of binding to a specific client.
- `client {}` fields: `tier`, `model`, `privacy`, `effort`, `default`, `extra { "k" "v" }` (provider-specific pass-through).
- Model strings are provider-prefixed (`claude:`, `codex:`, `cursor:`, `ollama:`, `local:`); bare names are rejected at compile-dispatch by `gaviero-core::swarm::backend::shared::validate_model_spec`.
- Scope glob patterns are enforced via [`gaviero_core::path_pattern::paths_overlap`](../gaviero-core/src/path_pattern.rs). Disjoint globs allowed; overlap within the same `loop { agents [...] }` group is allowed.
- Workflow blocks: `steps`, `depends_on`, `max_parallel`, `loop { until agent <judge> ... }` with `stability`, `iter_start`, `max_iterations`, `judge_timeout`, `strict_judge`, `branch_chain stacked`, `until command "..."`.

For exact field names and shapes, [`ast.rs`](src/ast.rs) is the source of truth.

## Conventions

- **Errors carry source spans.** Always propagate span info; never strip a `DslError` to its `Display` form before reporting.
- **Compile-time validation.** Scope overlaps and dependency cycles are checked at compile time. **Never** bypass these — runtime callers assume the plan is consistent.
- **Provider-neutral model strings.** Resolution to a backend happens in `gaviero-core` at dispatch; the DSL only validates the surface form.
- **Single-pass var substitution.** Do not re-run substitution; emit a diagnostic instead of silently iterating to a fixpoint.
- **Re-export of `CompiledPlan` is canonical.** Downstream crates use `gaviero_dsl::CompiledPlan`, not the path under `gaviero_core`.

## Rules

- **Never extend the AST without also extending [`ast.rs`](src/ast.rs).** Drift between AST and parser is the most common source of DSL bugs.
- **Tier-override files contain `tier` lines only** ([`tiers.rs`](src/tiers.rs)). Reject anything else with a diagnostic — do not silently ignore other items.
- **`include` resolution is whole-file.** Inline-string compilation paths (`compile`, `compile_with_vars`) must reject `include` and direct callers to `compile_file`.
- **Cycle detection happens in the resolver,** not the compiler. Compiler assumes a flat, deduplicated set of source files.

## Dependencies

- `gaviero-core` — `CompiledPlan`, `path_pattern::paths_overlap`, shared types.
- `logos 0.14` — lexer.
- `chumsky 0.12` — parser combinators.
- `miette 7` + `thiserror 2` — diagnostics.
- `tracing` — debug telemetry.

## See Also

- [README.md](README.md) — language reference, examples, full feature walkthrough.
- [ARCHITECTURE.md](ARCHITECTURE.md) — compilation pipeline, output types, scope validation, name resolution.
- [`../gaviero-core/CLAUDE.md`](../gaviero-core/CLAUDE.md) — `CompiledPlan` consumer side.
