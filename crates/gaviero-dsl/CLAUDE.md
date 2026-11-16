# gaviero-dsl

Compiler for `.gaviero` workflow scripts. Pipeline: include resolver → logos lexer → chumsky parser → AST → semantic analysis → `CompiledPlan` DAG.

## Build & Test

```bash
cargo test -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

Examples: **12** workflows + **4** model profiles (`default`, `claude`, `codex`, `mixed`) under [`examples/`](examples/).

## Architecture

**12 pub mods** — see [`src/lib.rs`](src/lib.rs):

| Module | Role |
|---|---|
| [`ast.rs`](src/ast.rs) | AST types. **Authoritative** DSL surface — read before extending docs or the parser. |
| [`lexer.rs`](src/lexer.rs) / [`parser.rs`](src/parser.rs) | Logos → chumsky → AST. |
| [`compiler.rs`](src/compiler.rs) | Semantic analysis → `CompiledPlan` (cycles, tier → profile client resolution, var substitution). Scope overlap is validated at swarm runtime. |
| [`workflow_params.rs`](src/workflow_params.rs) | Workflow `tiers { }` + `--tier` (`apply_agent_tiers`), list `param`s and roster expansion (`--param`). |
| [`reviewers.rs`](src/reviewers.rs) | Compat re-exports only; impl lives in `workflow_params`. |
| [`for_loops.rs`](src/for_loops.rs) | Private: `for <var> in <list> parallel/sequential` expansion (last pass of `workflow_params`); `for … in json` blocks are validated and lowered to their body agent + `WorkflowDecl::runtime_fors`, cloned at run time by core. |
| [`resolver.rs`](src/resolver.rs) | `include` graph; drives `compile_file`. |
| [`profile.rs`](src/profile.rs) | Merges a model profile (`client` + `tier`) into a resolved script; `reject_model_tables` refuses `client` / `tier <a> <c>` in scripts. |
| [`error.rs`](src/error.rs) | `DslError` / `DslErrors` (miette spans). |

**Public API** ([`lib.rs`](src/lib.rs)):

- `compile` / `compile_with_vars` (inline, take a profile) / `compile_file` — `override_vars`, `agent_tiers` (`--tier`), `override_params`.
- `compile_file_with_profile` — `compile_file` plus a [`SelectedProfile`](../gaviero-core/src/script_profiles.rs). Without a profile no tier is bound, so any script with agents fails to compile.
- `compile_file_with_models` — plus `&[TierOverride]` (`--tier-client TIER=CLIENT`, `--tier-effort TIER=EFFORT`): per-run rebinding of a tier to another client **of the same profile** and/or another effort (`profile::rebind_tiers`; an effort clones the client as `<client>__<tier>` so tiers sharing it are untouched; unknown client or effort = error; a missing tier is added only with a client).
- `inspect_profile` — a profile's clients (model, effort) and tier bindings; `ScriptInspection::agent_tiers(workflow)` — the agents a workflow runs and their default tier. Both back the SWARM form's Agents / Tiers rows.
- `workflow_execution_mode` / `peek_workflow_execution_mode` — `repo` vs `document` anchoring before full compile.
- `inspect_file` ([`inspect.rs`](src/inspect.rs)) — a script's inputs (workflows, params, vars, prompt use) without compiling; backs the TUI SWARM form.

**Precedence:** agent-level `vars {}` > CLI `--var` > script-level `vars {}`. Item vars (`for` / roster) are substituted before agent, CLI and script vars. Models — one way only: the profile (chosen, else the one named `default`) maps tiers to clients; an agent's tier is `--tier` > the workflow's `tiers { }` > its own required `tier`, and a list item's `tier` moves its clone; `--tier-client` / `--tier-effort` may point a tier at another of the profile's own clients and change its effort for one run. Scripts declare no clients or tier lines.

Exact field shapes: [`ast.rs`](src/ast.rs). Language reference: [README.md](README.md).

## Conventions

- **Errors carry source spans.** Never strip a `DslError` to bare `Display` before reporting.
- **Compile-time validation.** Dependency cycles and name resolution are checked at compile time. **Scope overlaps are not** — the DSL accepts overlapping `owned` globs; [`gaviero_core::swarm::validation::validate_scopes`](../gaviero-core/src/swarm/validation.rs) rejects them at execute time (see `dsl_does_not_catch_scope_overlap_swarm_validator_does` in [`tests/swarm_contract.rs`](tests/swarm_contract.rs)). Overlap within the same `loop { agents [...] }` group is allowed by that runtime validator.
- **Provider-neutral model strings.** Resolution happens in `gaviero-core` at dispatch. Prefixes: `claude:`, `codex:`, `cursor:`, `ollama:`, `local:`, `deepseek:`, `dsh:` ([`validate_model_spec`](../gaviero-core/src/swarm/backend/shared.rs)).
- **Termination is declared, not hoped for.** `agent { timeout <secs> }` (default 3600, `0` = off) bounds one dispatch including retries — the single reason a run is finite, since provider sessions only give up when their subprocess exits. `loop { irreconcilable_after N }` (default 2) stops a panel restating the same deadlock. Enforced in [`run_backend`](../gaviero-core/src/swarm/backend/runner.rs) and [`pipeline.rs`](../gaviero-core/src/swarm/pipeline.rs); outer cap via `gaviero-cli --run-timeout`.
- **`produces` is the output contract**, `scope { owned }` is the permission. Literal paths, not globs; compile-time vars substituted, `{{ITER}}` / `{{PREV_ITER}}` deferred to the runtime. Enforced in `gaviero-core` — per agent by [`run_backend`](../gaviero-core/src/swarm/backend/runner.rs) (missing → corrective retry → `Failed`), per loop pass by `assert_loop_agents_produced_output` ([`pipeline.rs`](../gaviero-core/src/swarm/pipeline.rs), missing → run aborts before the judge is dispatched). Roster clones get `{{REVIEWER_ID}}` substituted like `owned` does.
- **Single-pass var substitution.** Do not iterate to a fixpoint; emit a diagnostic instead.
- **Typed vars are script-level only.** `KEY file|dir ["default"]` ([`VarDecl`](src/ast.rs)) is a var, not a `param`, because run preparation ([`script_run`](../gaviero-core/src/swarm/script_run.rs)) reads paths from vars. A `file`'s existence is checked by the front ends (`require_file_vars`), never in the compiler, which has no run root. `dir` only selects the SWARM form's completion; at run time it is a string.
- **Canonical re-export:** downstream uses `gaviero_dsl::CompiledPlan`, not a path under core.

## Rules

- **Never extend the AST without updating [`ast.rs`](src/ast.rs).** Drift between AST and parser is the top DSL bug source.
- **`effort` is checked twice** ([`swarm/backend/effort.rs`](../gaviero-core/src/swarm/backend/effort.rs)): a misspelled name fails compilation (`is_known_effort`), and a wrong model + effort pair fails `preflight_plan_models` before any agent starts. Pairs are checked on the agents a run uses, so an unused client in a shared pool blocks nothing.
- **Scripts choose models by tier only.** Every agent declares `tier`; `client` blocks and `tier <a> <c>` lines are profile-only (rejected in scripts and their includes by `reject_model_tables`); there are no model-valued params; list items and runtime JSON items carry `tier`, never `model` / `effort`. Do not add a second way.
- **Profiles are closed model tables** ([`profile.rs`](src/profile.rs)): only `client` and `tier`, tiers point at the profile's own clients, and those clients are renamed on merge so they can never collide with the script's.
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
- [../../CLAUDE.md](../../CLAUDE.md) — workspace rules; **`plans/` + `research/` are history, not spec** (a newer prompt may contradict them and wins).
