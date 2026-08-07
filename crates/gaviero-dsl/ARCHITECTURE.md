# gaviero-dsl — Architecture

Compiler for `.gaviero` workflow scripts. Source → AST → [`CompiledPlan`](../gaviero-core/src/swarm/plan.rs) DAG for [`swarm::pipeline::execute`](../gaviero-core/src/swarm/pipeline.rs).

Conventions: [CLAUDE.md](CLAUDE.md). Language reference: [README.md](README.md).

---

## Topology

```
.gaviero source (+ optional --tiers-file / --var / --param)
        │
        ▼  compile_file only
   resolver::resolve (include graph, cycle-detect)
        │
        ▼
   lexer (logos) → parser (chumsky) → AST
        │
        ▼
   workflow_params::expand_…  (client + roster params)
        │
        ▼
   compiler::compile_ast_with_* → CompiledPlan → gaviero-core::swarm
```

Single synchronous library. Depends on [`gaviero-core`](../gaviero-core), logos, chumsky, miette, thiserror. No async; I/O only in `compile_file` / `load_tier_overrides` / `workflow_execution_mode`.

---

## Modules

**9 pub mods** ([`src/lib.rs`](src/lib.rs)):

| Module | Role |
|---|---|
| [`ast.rs`](src/ast.rs) | Authoritative DSL surface (`Script`, `Item`, decls, blocks) |
| [`lexer.rs`](src/lexer.rs) | Logos tokenizer |
| [`parser.rs`](src/parser.rs) | Chumsky → `Script` |
| [`compiler.rs`](src/compiler.rs) | Semantic analysis → `CompiledPlan`; `peek_workflow_execution_mode` |
| [`resolver.rs`](src/resolver.rs) | Transitive `include` resolution |
| [`workflow_params.rs`](src/workflow_params.rs) | `param` client + roster expansion (`--param`) |
| [`reviewers.rs`](src/reviewers.rs) | Compat re-exports of roster helpers |
| [`tiers.rs`](src/tiers.rs) | `load_tier_overrides` for `--tiers-file` |
| [`error.rs`](src/error.rs) | `DslError` / `DslErrors` (miette spans) |

Examples: **10** workflow scripts + **3** profiles under [`examples/`](examples) (`doc-claude`, `doc-codex`, `doc-cursor`).

---

## Abstractions

### AST ([`ast.rs`](src/ast.rs))

`Script { items: Vec<Item> }` with `Client`, `Agent`, `Workflow`, `Prompt`, `Vars`, `TierAlias`, `Include`. Field-level detail lives in the source — do not duplicate here.

### `CompiledPlan` / `ExecutionMode`

Re-exported from core. [`workflow_execution_mode`](src/lib.rs) / [`peek_workflow_execution_mode`](src/compiler.rs) resolve `execution repo|document` for workspace anchoring before full compile.

### Params

[`workflow_params`](src/workflow_params.rs) expands client params (`provider:model[@effort]`) into synthetic `__param_*` clients and roster params into per-reviewer agent clones before semantic compile.

### Tier overrides

[`load_tier_overrides`](src/tiers.rs) parses a profile that contains **only** `tier <alias> <client>` lines. CLI profile beats script/includes.

---

## Data Flow

```
source / entry_path
  │ compile_file → resolver::resolve (cycle reject, canonical dedup)
  ▼ lexer::lex → parser::parse → Script
  ▼ workflow_params::expand_workflow_params_in_script
  ▼ compiler::compile_ast_with_vars / compile_ast_with_sources
      index items → duplicate checks → select workflow
      merge vars (agent > CLI --var > script; AGENT/PROMPT reserved)
      resolve prompts + tier aliases (--tiers-file last)
      apply_vars
      build WorkUnit / LoopConfig / IterationConfig / VerificationConfig
      (scope overlap is NOT checked here — swarm `validate_scopes` at execute)
  ▼ CompiledPlan
```

Var precedence for `{{KEY}}`: reserved runtime (`PROMPT`, `AGENT`, planner injects) → agent `vars` → `--var` → script `vars`. Single-pass substitution.

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

Representative failures: unknown client/tier/prompt, duplicate name, circular `depends_on`, reserved var shadow, missing workflow, unknown `{{KEY}}`, include cycle, `include` under inline `compile`. Scope overlap is a swarm-runtime failure (`validate_scopes`), not a DSL diagnostic.

---

## API

```rust
// crates/gaviero-dsl/src/lib.rs
pub fn compile(
    source: &str, filename: &str,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_with_vars(
    source: &str, filename: &str,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    override_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_file(
    entry_path: &Path,
    workflow: Option<&str>, runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    override_tiers: &[(String, String)],
    override_params: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;

pub fn workflow_execution_mode(
    entry_path: &Path, workflow: Option<&str>,
) -> Result<ExecutionMode, miette::Report>;

pub use compiler::peek_workflow_execution_mode;
pub use tiers::load_tier_overrides;
pub use error::{DslError, DslErrors};
pub use gaviero_core::swarm::plan::{CompiledPlan, ExecutionMode};
```

Syntax cheat sheet and grammar surface: [README.md](README.md). Editor highlighting uses [`tree-sitter-gaviero`](../tree-sitter-gaviero) independently — this crate does not depend on tree-sitter.
