# gaviero-dsl

Compiler for `.gaviero` workflow scripts. Pipeline: include resolver → logos lexer → chumsky parser → AST → semantic analysis → `CompiledPlan` DAG.

## Build & Test

```bash
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

Examples: **9** workflows + **3** tier profiles (`doc-claude`, `doc-codex`, `doc-cursor`) under [`examples/`](examples/).

## Architecture

**9 pub mods** — see [`src/lib.rs`](src/lib.rs):

| Module | Role |
|---|---|
| [`ast.rs`](src/ast.rs) | AST types. **Authoritative** DSL surface — read before extending docs or the parser. |
| [`lexer.rs`](src/lexer.rs) / [`parser.rs`](src/parser.rs) | Logos → chumsky → AST. |
| [`compiler.rs`](src/compiler.rs) | Semantic analysis → `CompiledPlan` (cycles, tier resolution, var substitution). Scope overlap is validated at swarm runtime. |
| [`workflow_params.rs`](src/workflow_params.rs) | `param` materialization: client params + roster expansion (`--param`). |
| [`reviewers.rs`](src/reviewers.rs) | Compat re-exports only; impl lives in `workflow_params`. |
| [`resolver.rs`](src/resolver.rs) | `include` graph; drives `compile_file`. |
| [`tiers.rs`](src/tiers.rs) | `--tiers-file` loader (`tier` lines only). |
| [`error.rs`](src/error.rs) | `DslError` / `DslErrors` (miette spans). |

**Public API** ([`lib.rs`](src/lib.rs)):

- `compile` / `compile_with_vars` / `compile_file` — `override_vars`, `override_tiers`, `override_params`.
- `load_tier_overrides` — backs `--tiers-file`.
- `workflow_execution_mode` / `peek_workflow_execution_mode` — `repo` vs `document` anchoring before full compile.

**Precedence:** agent-level `vars {}` > CLI `--var` > script-level `vars {}`. Tier: `--tiers-file` > script/includes `tier` lines.

Exact field shapes: [`ast.rs`](src/ast.rs). Language reference: [README.md](README.md).

## Conventions

- **Errors carry source spans.** Never strip a `DslError` to bare `Display` before reporting.
- **Compile-time validation.** Dependency cycles and name resolution are checked at compile time. **Scope overlaps are not** — the DSL accepts overlapping `owned` globs; [`gaviero_core::swarm::validation::validate_scopes`](../gaviero-core/src/swarm/validation.rs) rejects them at execute time (see `dsl_does_not_catch_scope_overlap_swarm_validator_does` in [`tests/swarm_contract.rs`](tests/swarm_contract.rs)). Overlap within the same `loop { agents [...] }` group is allowed by that runtime validator.
- **Provider-neutral model strings.** Resolution happens in `gaviero-core` at dispatch. Prefixes: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:` ([`validate_model_spec`](../gaviero-core/src/swarm/backend/shared.rs)).
- **Single-pass var substitution.** Do not iterate to a fixpoint; emit a diagnostic instead.
- **Canonical re-export:** downstream uses `gaviero_dsl::CompiledPlan`, not a path under core.

## Rules

- **Never extend the AST without updating [`ast.rs`](src/ast.rs).** Drift between AST and parser is the top DSL bug source.
- **Tier-override files contain `tier` lines only** ([`tiers.rs`](src/tiers.rs)). Reject other items with a diagnostic.
- **`include` is whole-file.** Inline paths (`compile`, `compile_with_vars`) must reject `include` and point callers to `compile_file`.
- **Cycle detection lives in the resolver,** not the compiler.

## Dependencies

- `gaviero-core` — `CompiledPlan`, shared types. (Scope overlap uses core `path_pattern` at **swarm runtime**, not in this crate.)
- `logos 0.14` — lexer.
- `chumsky 0.12` — parser.
- `miette 7` + `thiserror 2` — diagnostics.
- `tracing` — debug telemetry.
- Dev: `tempfile 3`.

## See Also

- [README.md](README.md) — language reference + examples.
- [ARCHITECTURE.md](ARCHITECTURE.md) — compilation pipeline, output types, name resolution.
- [`../gaviero-core/CLAUDE.md`](../gaviero-core/CLAUDE.md) — `CompiledPlan` consumer side.
