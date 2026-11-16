# gaviero-dsl — Architecture

Compiler for `.gaviero` workflow scripts. Source → AST → [`CompiledPlan`](../gaviero-core/src/swarm/plan.rs) DAG for [`swarm::pipeline::execute`](../gaviero-core/src/swarm/pipeline.rs).

Conventions: [CLAUDE.md](CLAUDE.md). Language reference: [README.md](README.md).

---

## Topology

```
.gaviero source (+ model profile, optional --var / --tier / --param)
        │
        ▼  compile_file only
   resolver::resolve (include graph, cycle-detect)
        │
        ▼
   lexer (logos) → parser (chumsky) → AST
        │
        ▼  profile::reject_model_tables, then profile::apply_profile
   workflow_params::apply_agent_tiers (tiers { } + --tier)
        │
        ▼
   workflow_params::expand_…  (roster params, then for_loops::expand_for_blocks)
        │
        ▼
   compiler::compile_ast_with_* → CompiledPlan → gaviero-core::swarm
```

Single synchronous library. Depends on [`gaviero-core`](../gaviero-core), logos, chumsky, miette, thiserror. No async; I/O only in `compile_file*`, profile loading and `workflow_execution_mode`.

---

## Modules

**10 pub mods** ([`src/lib.rs`](src/lib.rs)):

| Module | Role |
|---|---|
| [`ast.rs`](src/ast.rs) | Authoritative DSL surface (`Script`, `Item`, decls, blocks) |
| [`lexer.rs`](src/lexer.rs) | Logos tokenizer |
| [`parser.rs`](src/parser.rs) | Chumsky → `Script` |
| [`compiler.rs`](src/compiler.rs) | Semantic analysis → `CompiledPlan`; `peek_workflow_execution_mode` |
| [`resolver.rs`](src/resolver.rs) | Transitive `include` resolution |
| [`workflow_params.rs`](src/workflow_params.rs) | Workflow `tiers { }` + `--tier`, list params and roster expansion (`--param`) |
| [`reviewers.rs`](src/reviewers.rs) | Compat re-exports of roster helpers |
| [`for_loops.rs`](src/for_loops.rs) | Private: `for … parallel/sequential` expansion into per-item clones |
| [`profile.rs`](src/profile.rs) | Rejects `client` / tier lines in scripts; merges the model profile (`--profile`, the SWARM form, or `default`) |
| [`inspect.rs`](src/inspect.rs) | A script's inputs (workflows, list params, vars, tiers used) for the SWARM form |
| [`error.rs`](src/error.rs) | `DslError` / `DslErrors` (miette spans) |

Examples: **12** workflow scripts + **4** model profiles under [`examples/`](examples) (`default`, `claude`, `codex`, `mixed`).

---

## Abstractions

### AST ([`ast.rs`](src/ast.rs))

`Script { items: Vec<Item> }` with `Client`, `Agent`, `Workflow`, `Prompt`, `Vars`, `TierAlias`, `Include`. `Client` and `TierAlias` come only from the model profile: the public entry points reject them in a script. Field-level detail lives in the source — do not duplicate here.

### `CompiledPlan` / `ExecutionMode`

Re-exported from core. [`workflow_execution_mode`](src/lib.rs) / [`peek_workflow_execution_mode`](src/compiler.rs) resolve `execution repo|document` for workspace anchoring before full compile.

Besides the DAG and `LoopConfig`s the compiler fills:

- `runtime_fors` — one `RuntimeForOp { unit_id, var, source, max_items }` per `for … in json` step; `unit_id` names a template node in the graph whose `{{<var>.*}}` placeholders survive compile.
- `tier_bindings` — every tier of the profile resolved to its client's model / effort / extra / privacy / tier, so runtime items (`for … in json` items, spawn-manifest workers) can name a `tier`.
- `budget` — the workflow's `budget { wall usd }` (`RunBudget`).

`LoopConfig::agent_ids` is in dependency order: `depends_on` between agents of the same loop sorts them (stably). A workflow may contain at most one `loop`.

### Params

[`workflow_params`](src/workflow_params.rs) first applies the selected workflow's `tiers { agent tier }` block and the `--tier` overrides to the agents' declared tiers (`apply_agent_tiers`), then expands roster params into per-reviewer agent clones before semantic compile. A roster entry's `tier` replaces its clones' tier. Params are lists only; no param chooses a model.

Its last pass, [`for_loops::expand_for_blocks`](src/for_loops.rs), expands `for <var> in <list> parallel|sequential { … }`: one clone `<item-id>-<agent>` per item and body agent, item values in `AgentDecl::item_vars` (`<var>.id`, `<var>.<field>`, `<var>.index`, …), the sequential chain as `depends_on`, and fan-in `depends_on [body]` rewritten to every clone. A `for … parallel` inside a `loop` appends its clones to the loop's agents; when the step right before the loop is a top-level `for` over the same list, its clones become `LoopBlock::baseline_units`, emitted as `LoopConfig::baseline_units` (scope-group and resume pairing in core, alongside the roster's `-init`/`-refine` suffix rule).

`for <var> in json "<path>" …` blocks are **not** expanded at compile time: expansion validates them, replaces each with its single body agent (a plain step, or a loop agent), and records it in `WorkflowDecl::runtime_fors`, which the compiler emits as `CompiledPlan::runtime_fors`. The core pipeline ([`swarm/runtime_for.rs`](../gaviero-core/src/swarm/runtime_for.rs)) reads the list and clones the template when the step runs.

### Models

One way only. The model profile maps tiers to clients ([`profile.rs`](src/profile.rs)); every agent declares a tier; an agent's tier is `--tier` > the workflow's `tiers { }` > its own; a list item's `tier` moves its clone. `compile_agent` resolves `tier_ref` → profile tier → client; an agent without a tier, or a tier the profile lacks, is a compile error.

---

## Data Flow

```
source / entry_path
  │ compile_file → resolver::resolve (cycle reject, canonical dedup)
  ▼ lexer::lex → parser::parse → Script
  ▼ profile::reject_model_tables → profile::apply_profile
  ▼ workflow_params::apply_agent_tiers → expand_workflow_params_in_script (roster → for_loops::expand_for_blocks)
  ▼ compiler::compile_ast_with_vars / compile_ast_with_sources
      index items → duplicate checks → select workflow
      merge vars (agent > CLI --var > script; AGENT/PROMPT reserved)
      resolve prompts + agent tier → profile client
      apply_vars
      build WorkUnit / LoopConfig / IterationConfig / VerificationConfig
      (scope overlap is NOT checked here — swarm `validate_scopes` at execute)
  ▼ CompiledPlan
```

Var precedence for `{{KEY}}`: reserved runtime (`PROMPT`, `AGENT`, planner injects) → agent `vars` → `--var` → script `vars`. Item vars (`AgentDecl::item_vars`, from `for` / roster expansion) are substituted first of all, so placeholders inside their values (`{{OUT_DIR}}` in a peer block) still resolve. Single pass per layer.

---

## Concurrency

None. Single-threaded, no locks. Safe to call from any async task as a pure CPU/IO burst.

---

## Error Handling

All stages produce `miette::Report` wrapping [`DslErrors(Vec<DslError>)`](src/error.rs):

```rust
enum DslError { Lex {..}, Parse {..}, Compile {..}, Resolve {..} }
```

Spans point at the originating file (multi-file via `NamedSource` in `compile_file`). No panics on the public API.

Representative failures: agent without a tier, tier the profile lacks, `client` / tier line in a script, unknown prompt, duplicate name, circular `depends_on`, reserved var shadow, missing workflow, unknown `{{KEY}}`, include cycle, `include` under inline `compile`. Scope overlap is a swarm-runtime failure (`validate_scopes`), not a DSL diagnostic.

---

## API

```rust
// crates/gaviero-dsl/src/lib.rs
pub fn compile(
    source: &str, filename: &str, profile: Option<&SelectedProfile>,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_with_vars(
    source: &str, filename: &str, profile: Option<&SelectedProfile>,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],     // --tier AGENT=TIER
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_file_with_profile(
    entry_path: &Path, profile: Option<&SelectedProfile>,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    agent_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;
// compile_file(...) = compile_file_with_profile(.., None, ..)

pub fn workflow_execution_mode(
    entry_path: &Path, workflow: Option<&str>,
) -> Result<ExecutionMode, miette::Report>;

pub use compiler::peek_workflow_execution_mode;
pub use error::{DslError, DslErrors};
pub use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode};
```

Syntax cheat sheet and grammar surface: [README.md](README.md). Editor highlighting uses [`tree-sitter-gaviero`](../tree-sitter-gaviero) independently — this crate does not depend on tree-sitter.
